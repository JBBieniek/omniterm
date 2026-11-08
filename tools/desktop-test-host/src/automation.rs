//! Separate Windows test executable only; never linked by the production app.

use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{self, BufRead, Read, Write};
use std::sync::mpsc;
use std::time::Duration;
use tauri::Manager;
use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
use windows::core::HSTRING;
use windows::Win32::Foundation::{SetHandleInformation, HANDLE_FLAGS, HANDLE_FLAG_INHERIT};
use windows::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_PIPE};
use windows::Win32::System::Console::{GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};

const MAX_REQUEST: usize = 4096;
const MAX_RESPONSE: usize = 8 * 1024 * 1024;

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Inspect,
    Screenshot,
    Click { x: u32, y: u32 },
    Key { key: Key },
    Revoke,
    Close,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Key {
    Tab,
    Enter,
    Escape,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    hello: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    token: String,
    command: Command,
}

struct Session {
    token: Option<String>,
}

impl Session {
    fn from_hello(bytes: &[u8]) -> Result<Self, &'static str> {
        let hello: Hello = serde_json::from_slice(bytes).map_err(|_| "invalid_handshake")?;
        if hello.hello.len() != 64 || !hello.hello.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid_handshake");
        }
        Ok(Self {
            token: Some(hello.hello),
        })
    }

    fn authorize(&mut self, bytes: &[u8]) -> Result<Command, &'static str> {
        let request: Request = serde_json::from_slice(bytes).map_err(|_| "invalid_request")?;
        if self.token.as_deref() != Some(request.token.as_str()) {
            return Err("unauthorized");
        }
        if request.command == Command::Revoke {
            self.token = None;
        }
        Ok(request.command)
    }
}

fn read_frame(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, &'static str> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_REQUEST + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| "read_failed")?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.len() > MAX_REQUEST {
        return Err("request_too_large");
    }
    if bytes.last() != Some(&b'\n') {
        return Err("truncated_request");
    }
    Ok(Some(bytes))
}

/// Check inherited anonymous pipes before creating WebView children. No socket,
/// token in argv/environment, named discovery endpoint, or renderer grant.
pub(super) fn prepare_pipes() -> Result<(), &'static str> {
    for kind in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE] {
        // SAFETY: handles come from Windows; neither is closed or retained here.
        unsafe {
            let handle = GetStdHandle(kind).map_err(|_| "automation requires private pipes")?;
            if GetFileType(handle) != FILE_TYPE_PIPE {
                return Err("automation requires private pipes");
            }
            SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0))
                .map_err(|_| "could not protect automation pipe inheritance")?;
        }
    }
    Ok(())
}

pub(super) fn start(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let mut input = io::stdin().lock();
        let mut output = io::stdout().lock();
        let result = serve(&app, &mut input, &mut output);
        if let Err(error) = result {
            let _ = reply(&mut output, Err(error));
        }
        // EOF, malformed handshake or oversized input permanently closes this
        // channel. The app stays open; only authenticated Close exits the app.
    });
}

fn reply(output: &mut impl Write, result: Result<Value, &str>) -> Result<(), &'static str> {
    let value = match result {
        Ok(result) => json!({ "result": result }),
        Err(error) => json!({ "error": error }),
    };
    let mut bytes = serde_json::to_vec(&value).map_err(|_| "encode_failed")?;
    if bytes.len() > MAX_RESPONSE {
        return Err("response_too_large");
    }
    bytes.push(b'\n');
    output
        .write_all(&bytes)
        .and_then(|()| output.flush())
        .map_err(|_| "write_failed")
}

fn serve(
    app: &tauri::AppHandle,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<(), &'static str> {
    let hello = read_frame(input)?.ok_or("missing_handshake")?;
    let mut session = Session::from_hello(&hello)?;
    reply(
        output,
        Ok(json!({ "protocol": 1, "transport": "private-stdio" })),
    )?;
    while let Some(bytes) = read_frame(input)? {
        let command = match session.authorize(&bytes) {
            Ok(command) => command,
            Err(error) => {
                reply(output, Err(error))?;
                continue;
            }
        };
        match command {
            Command::Inspect => reply(output, inspect(app))?,
            Command::Screenshot => reply(output, screenshot(app))?,
            Command::Click { x, y } => reply(output, click(app, x, y))?,
            Command::Key { key } => reply(output, key_event(app, key))?,
            Command::Revoke => reply(output, Ok(json!({ "revoked": true })))?,
            Command::Close => {
                reply(output, Ok(json!({ "closing": true })))?;
                app.exit(0);
                return Ok(());
            }
        }
    }
    Ok(())
}

fn window(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, &'static str> {
    let window = app.get_webview_window("main").ok_or("window_missing")?;
    let url = window.url().map_err(|_| "url_unavailable")?;
    if !((url.scheme() == "http" && url.host_str() == Some("tauri.localhost"))
        || (url.scheme() == "tauri" && url.host_str() == Some("localhost")))
    {
        return Err("bundled_content_required");
    }
    Ok(window)
}

// Fixed observations only. No caller-supplied script, selector, path, URL or CDP
// method; no test command is registered in Tauri's renderer IPC handler.
const OBSERVE: &str = r#"(async () => {
  // Test-host-only counters prove engine input delivery without recording text.
  if (!globalThis.__omniterm_test_events) {
    const events = { clicks: 0, key_downs: 0, click_trusted: false, key_trusted: false };
    globalThis.__omniterm_test_events = events;
    document.addEventListener('click', event => {
      events.clicks = Math.min(events.clicks + 1, 1000000);
      events.click_trusted = event.isTrusted;
    }, true);
    document.addEventListener('keydown', event => {
      events.key_downs = Math.min(events.key_downs + 1, 1000000);
      events.key_trusted = event.isTrusted;
    }, true);
  }
  return {
  origin: location.origin,
  rendered_revision: document.querySelector('.application')?.dataset.renderedRevision ?? null,
  status: document.querySelector('[data-testid="workspace-status"]')?.textContent?.trim().slice(0, 256) ?? null,
  block_count: document.querySelector('[data-testid="block-count"]')?.textContent?.trim().slice(0, 256) ?? null,
  grid: document.querySelector('[data-testid="grid-dimensions"]')?.textContent?.trim().slice(0, 256) ?? null,
  cell: document.querySelector('[data-testid="cell-size"]')?.textContent?.trim().slice(0, 256) ?? null,
  focused: document.hasFocus(),
  input_events: {...globalThis.__omniterm_test_events},
  test_command_rejected: await window.__TAURI_INTERNALS__.invoke('automation_inspect').then(() => false, () => true)
}; })()"#;

fn inspect(app: &tauri::AppHandle) -> Result<Value, &'static str> {
    let window = window(app)?;
    let rendered = protocol(
        &window,
        "Runtime.evaluate",
        json!({
            "expression": OBSERVE, "awaitPromise": true, "returnByValue": true
        }),
    )?;
    let rendered = rendered
        .pointer("/result/value")
        .ok_or("observation_failed")?;
    let layout = omniterm_desktop::snapshot::LayoutSnapshotV1::from_workspace(
        &app.state::<omniterm_core::Workspace>(),
        0,
    );
    let size = window.inner_size().map_err(|_| "window_unavailable")?;
    Ok(json!({ "layout": layout, "rendered": rendered, "window": {
        "title": window.title().map_err(|_| "window_unavailable")?,
        "width": size.width, "height": size.height,
        "scale_factor": window.scale_factor().map_err(|_| "window_unavailable")?,
        "visible": window.is_visible().map_err(|_| "window_unavailable")?
    }}))
}

fn screenshot(app: &tauri::AppHandle) -> Result<Value, &'static str> {
    let window = window(app)?;
    let size = window.inner_size().map_err(|_| "window_unavailable")?;
    if size.width > 4096 || size.height > 4096 {
        return Err("viewport_too_large");
    }
    protocol(
        &window,
        "Page.captureScreenshot",
        json!({ "format": "png", "captureBeyondViewport": false }),
    )
}

fn validate_point(x: u32, y: u32, width: f64, height: f64) -> Result<(), &'static str> {
    if f64::from(x) >= width || f64::from(y) >= height {
        return Err("point_outside_webview");
    }
    Ok(())
}

fn click(app: &tauri::AppHandle, x: u32, y: u32) -> Result<Value, &'static str> {
    let window = window(app)?;
    let size = window
        .inner_size()
        .map_err(|_| "window_unavailable")?
        .to_logical::<f64>(window.scale_factor().map_err(|_| "window_unavailable")?);
    validate_point(x, y, size.width, size.height)?;
    for (event, buttons) in [("mousePressed", 1), ("mouseReleased", 0)] {
        protocol(
            &window,
            "Input.dispatchMouseEvent",
            json!({
                "type": event, "x": x, "y": y, "button": "left", "buttons": buttons, "clickCount": 1
            }),
        )?;
    }
    Ok(json!({ "clicked": true }))
}

fn key_event(app: &tauri::AppHandle, key: Key) -> Result<Value, &'static str> {
    let window = window(app)?;
    let (name, code) = match key {
        Key::Tab => ("Tab", 9),
        Key::Enter => ("Enter", 13),
        Key::Escape => ("Escape", 27),
    };
    for event in ["keyDown", "keyUp"] {
        protocol(
            &window,
            "Input.dispatchKeyEvent",
            json!({
                "type": event, "key": name, "code": name,
                "windowsVirtualKeyCode": code, "nativeVirtualKeyCode": code
            }),
        )?;
    }
    Ok(json!({ "key_sent": true }))
}

fn protocol(
    window: &tauri::WebviewWindow,
    method: &'static str,
    params: Value,
) -> Result<Value, &'static str> {
    let (sender, receiver) = mpsc::sync_channel(1);
    window
        .with_webview(move |webview| {
            let completed_sender = sender.clone();
            let callback = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                move |status, result| {
                    let result = status.map_err(|_| "native_call_failed").and_then(|()| {
                        if result.len() > MAX_RESPONSE {
                            return Err("response_too_large");
                        }
                        serde_json::from_str(&result).map_err(|_| "invalid_native_response")
                    });
                    let _ = completed_sender.try_send(result);
                    Ok(())
                },
            ));
            // SAFETY: Tauri runs this closure on its WebView STA thread; COM objects
            // and string parameters stay alive for the synchronous dispatch call.
            let result = unsafe {
                webview.controller().CoreWebView2().and_then(|core| {
                    core.CallDevToolsProtocolMethod(
                        &HSTRING::from(method),
                        &HSTRING::from(params.to_string()),
                        &callback,
                    )
                })
            };
            if result.is_err() {
                let _ = sender.try_send(Err("native_dispatch_failed"));
            }
        })
        .map_err(|_| "webview_unavailable")?;
    receiver
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "native_call_timeout")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn steering_is_typed_and_stays_inside_the_webview() {
        let mut session =
            Session::from_hello(format!(r#"{{"hello":"{TOKEN}"}}"#).as_bytes()).unwrap();
        assert_eq!(
            session
                .authorize(
                    format!(r#"{{"token":"{TOKEN}","command":{{"click":{{"x":12,"y":34}}}}}}"#)
                        .as_bytes()
                )
                .unwrap(),
            Command::Click { x: 12, y: 34 }
        );
        assert_eq!(
            session
                .authorize(
                    format!(r#"{{"token":"{TOKEN}","command":{{"key":{{"key":"tab"}}}}}}"#)
                        .as_bytes()
                )
                .unwrap(),
            Command::Key { key: Key::Tab }
        );
        for command in [
            r#"{"click":{"x":-1,"y":0}}"#,
            r#"{"click":{"x":1.5,"y":0}}"#,
            r#"{"click":{"x":0,"y":0,"script":"bad"}}"#,
            r#"{"key":{"key":"host_shortcut"}}"#,
        ] {
            assert!(session
                .authorize(format!(r#"{{"token":"{TOKEN}","command":{command}}}"#).as_bytes())
                .is_err());
        }
        assert!(validate_point(0, 0, 1024.0, 700.0).is_ok());
        assert!(validate_point(1023, 699, 1024.0, 700.0).is_ok());
        assert!(validate_point(1024, 0, 1024.0, 700.0).is_err());
        assert!(validate_point(0, 700, 1024.0, 700.0).is_err());
        assert!(validate_point(u32::MAX, 0, 1024.0, 700.0).is_err());
    }

    #[test]
    fn handshake_requires_exact_random_token_shape() {
        assert!(Session::from_hello(format!(r#"{{"hello":"{TOKEN}"}}"#).as_bytes()).is_ok());
        for hello in [r#"{}"#, r#"{"hello":"short"}"#, r#"{"hello":1}"#] {
            assert!(Session::from_hello(hello.as_bytes()).is_err());
        }
        assert!(
            Session::from_hello(format!(r#"{{"hello":"{TOKEN}","extra":1}}"#).as_bytes()).is_err()
        );
        assert!(
            Session::from_hello(format!(r#"{{"hello":"{}"}}"#, "z".repeat(64)).as_bytes()).is_err()
        );
    }

    #[test]
    fn unauthenticated_unknown_and_revoked_requests_cannot_inspect() {
        let mut session =
            Session::from_hello(format!(r#"{{"hello":"{TOKEN}"}}"#).as_bytes()).unwrap();
        for request in [
            r#"{"command":"inspect"}"#.to_owned(),
            r#"{"token":"wrong","command":"inspect"}"#.to_owned(),
            format!(r#"{{"token":"{TOKEN}","command":"execute","script":"anything"}}"#),
            format!(r#"{{"token":"{TOKEN}","command":"inspect","path":"secret"}}"#),
        ] {
            assert!(session.authorize(request.as_bytes()).is_err());
        }
        let inspect = format!(r#"{{"token":"{TOKEN}","command":"inspect"}}"#);
        assert_eq!(
            session.authorize(inspect.as_bytes()).unwrap(),
            Command::Inspect
        );
        assert_eq!(
            session
                .authorize(format!(r#"{{"token":"{TOKEN}","command":"revoke"}}"#).as_bytes())
                .unwrap(),
            Command::Revoke
        );
        assert!(session.authorize(inspect.as_bytes()).is_err());
        assert!(session
            .authorize(format!(r#"{{"token":"{TOKEN}","command":"close"}}"#).as_bytes())
            .is_err());
    }

    #[test]
    fn frames_bound_memory_and_require_complete_lines() {
        let mut input = Cursor::new(b"{}\n{}\n");
        assert_eq!(read_frame(&mut input).unwrap().unwrap(), b"{}\n");
        assert_eq!(read_frame(&mut input).unwrap().unwrap(), b"{}\n");
        assert!(read_frame(&mut input).unwrap().is_none());
        assert!(read_frame(&mut Cursor::new(b"{}")).is_err());
        let mut huge = Cursor::new(vec![b'x'; MAX_REQUEST * 10]);
        assert!(read_frame(&mut huge).is_err());
        assert_eq!(huge.position(), (MAX_REQUEST + 1) as u64);
        let mut boundary = vec![b' '; MAX_REQUEST];
        boundary[MAX_REQUEST - 1] = b'\n';
        assert_eq!(
            read_frame(&mut Cursor::new(boundary))
                .unwrap()
                .unwrap()
                .len(),
            MAX_REQUEST
        );
    }
}
