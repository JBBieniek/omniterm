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
    Click {
        x: u32,
        y: u32,
    },
    Pointer {
        phase: PointerPhase,
        button: PointerButton,
        x: u32,
        y: u32,
    },
    Resize {
        width: u32,
        height: u32,
    },
    ShortcutGuards,
    AdvanceLayout,
    CancelPan {
        lost_capture: bool,
    },
    Key {
        key: Key,
    },
    Text {
        text: String,
    },
    Paste {
        text: String,
    },
    RequestClose,
    Revoke,
    Close,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Key {
    Tab,
    Enter,
    Escape,
    Interrupt,
    ZoomIn,
    ZoomPlus,
    ZoomOut,
    ZoomReset,
    NumpadAdd,
    NumpadSubtract,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    ResizeLeft,
    ResizeRight,
    ResizeUp,
    ResizeDown,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PointerPhase {
    Down,
    Move,
    Up,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PointerButton {
    Left,
    Middle,
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
            Command::Pointer {
                phase,
                button,
                x,
                y,
            } => reply(output, pointer(app, phase, button, x, y))?,
            Command::Resize { width, height } => reply(output, resize(app, width, height))?,
            Command::ShortcutGuards => reply(output, shortcut_guards(app))?,
            Command::AdvanceLayout => reply(output, advance_layout(app))?,
            Command::CancelPan { lost_capture } => reply(output, cancel_pan(app, lost_capture))?,
            Command::Key { key } => reply(output, key_event(app, key))?,
            Command::Text { text } => reply(output, insert_text(app, &text))?,
            Command::Paste { text } => reply(output, paste_text(app, &text))?,
            Command::RequestClose => reply(
                output,
                window(app).and_then(|w| {
                    w.close().map_err(|_| "close_failed")?;
                    Ok(json!({"close_requested": true}))
                }),
            )?,
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
  const app = document.querySelector('.application');
  const world = document.querySelector('[data-testid="world"]');
  const viewport = document.querySelector('.board-viewport');
  const preview = document.querySelector('[data-testid="placement-preview"]');
  const matrix = world ? new DOMMatrix(getComputedStyle(world).transform) : null;
  const bounds = element => {
    if (!element) return null;
    const rect = element.getBoundingClientRect();
    return {x: rect.x, y: rect.y, width: rect.width, height: rect.height};
  };
  const control = id => {
    const element = document.querySelector(`[data-testid="${id}"]`);
    return element ? {...bounds(element), disabled: element.disabled} : null;
  };
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
    document.addEventListener('pointerdown', event => { events.pointer_id = event.pointerId; }, true);
  }
  return {
  origin: location.origin,
  rendered_revision: app?.dataset.renderedRevision ?? null,
  selected_id: app?.dataset.selectedBlockId || null,
  busy: app?.dataset.busy === 'true',
  error: document.querySelector('[data-testid="command-error"]')?.textContent?.trim().slice(0, 1024) ?? '',
  controls: { title: control('placeholder-title'), add: control('add-placeholder'), remove: control('remove-placement'),
    new_terminal: control('new-terminal'), reopen: control('reopen-terminal'), terminate: control('terminate-session'),
    cancel_quit: control('cancel-quit'), confirm_quit: control('confirm-quit'),
    show_text: control('show-opened-text'),
    show_git: control('show-opened-git'),
    pan: control('pan-tool'), zoom_in: control('zoom-in'), zoom_out: control('zoom-out'), reset: control('reset-view') },
  camera: matrix ? {zoom: matrix.a * 100, origin: {x: -matrix.e / matrix.a, y: -matrix.f / matrix.d}} : null,
  viewport: viewport ? {...bounds(viewport), scroll_x: viewport.scrollLeft, scroll_y: viewport.scrollTop} : null,
  page: {dpr: devicePixelRatio, scale: visualViewport?.scale, toolbar: bounds(document.querySelector('.toolbar'))},
  pan_tool: document.querySelector('[data-testid="pan-tool"]')?.getAttribute('aria-pressed') === 'true',
  panning: viewport?.classList.contains('panning') ?? false,
  active: {test_id: document.activeElement?.dataset.testid ?? null, block_id: document.activeElement?.closest('[data-block-id]')?.dataset.blockId ?? null,
    focus_visible: document.activeElement?.matches(':focus-visible') ?? false},
  placement_preview: preview ? {viewport: bounds(preview), invalid: preview.classList.contains('invalid'),
    text: preview.textContent?.trim().slice(0, 256), left:parseFloat(preview.style.left), top:parseFloat(preview.style.top),
    width:parseFloat(preview.style.width), height:parseFloat(preview.style.height)} : null,
  world: world ? {width: world.offsetWidth, height: world.offsetHeight, child_count: world.children.length,
    background_size: getComputedStyle(world).backgroundSize, background_image: getComputedStyle(world).backgroundImage} : null,
  blocks: [...document.querySelectorAll('[data-block-id]')].slice(0, 256).map(element => ({
    id: element.dataset.blockId, title: element.querySelector('[data-testid="block-title"]')?.textContent ?? '',
    selected: element.querySelector('.block-select')?.getAttribute('aria-pressed') === 'true', viewport: bounds(element),
    header: bounds(element.querySelector('[data-drag-block]')), resize: bounds(element.querySelector('[data-resize-block]')),
    left: parseFloat(element.style.left), top: parseFloat(element.style.top),
    width: parseFloat(element.style.width), height: parseFloat(element.style.height),
    injected_elements: element.querySelectorAll('script,img,iframe,svg').length
  })),
  terminals: [...document.querySelectorAll('[data-session-id]')].slice(0, 4).map(element => ({
    session_id: element.dataset.sessionId, state: element.dataset.terminalState,
    rows: Number(element.dataset.terminalRows), cols: Number(element.dataset.terminalCols),
    input: bounds(element.querySelector('textarea')),
    text: [...element.querySelectorAll('.terminal-row')].map(row => row.textContent).join('\n'),
    status: element.querySelector('.terminal-status')?.textContent,
    error: element.querySelector('.terminal-error')?.textContent ?? ''
  })),
  quit_dialog: !!document.querySelector('dialog[open]'),
  git_views: [...document.querySelectorAll('[data-git-task-id]')].slice(0,8).map(element => ({
    task_id:element.dataset.gitTaskId, state:element.dataset.gitState, viewport:bounds(element),
    counts:element.querySelector('[data-testid="git-counts"]')?.textContent ?? null,
    interpretation:element.querySelector('.git-interpretation')?.textContent ?? null,
    outcome:element.querySelector('[data-testid="git-outcome"]')?.textContent ?? null,
    gap:element.querySelector('[data-testid="git-gap"]')?.textContent ?? null,
    raw:element.querySelector('[data-testid="git-raw"]')?.textContent?.slice(0,8192) ?? null,
    source:element.querySelector('.git-source')?.textContent ?? null,
    refresh:bounds(element.querySelector('[data-testid="refresh-git"]')),
    cancel:bounds(element.querySelector('[data-testid="cancel-git"]')),
    toggle_raw:bounds(element.querySelector('[data-testid="raw-git"]')),
    injected_elements:element.querySelectorAll('script,img,iframe,svg').length
  })),
  git_reopen: [...document.querySelectorAll('[data-reopen-git]')].slice(0,8).map(e => ({task_id:e.dataset.reopenGit,...bounds(e)})),
  text_views: [...document.querySelectorAll('[data-text-block-id]')].slice(0,8).map(element => ({
    id: element.dataset.textBlockId, source:element.querySelector('.text-source')?.textContent ?? '',
    content:element.querySelector('[data-testid="text-content"]')?.textContent?.slice(0,4096) ?? null,
    length:element.querySelector('[data-testid="text-content"]')?.textContent?.length ?? null,
    viewport:bounds(element), injected_elements:element.querySelectorAll('script,img,iframe,svg').length
  })),
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
    let layout = app
        .state::<std::sync::Mutex<omniterm_desktop::commands::WorkspaceHost>>()
        .lock()
        .map_err(|_| "workspace_unavailable")?
        .snapshot();
    let size = window.inner_size().map_err(|_| "window_unavailable")?;
    let runtime = app.state::<omniterm_desktop::terminal::TerminalHost>();
    let sessions = runtime.list();
    let screens = sessions
        .iter()
        .map(|session| {
            runtime
                .screen(session.session_id.into(), None)
                .map_err(|_| "terminal_unavailable")
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(
        json!({ "layout": layout, "rendered": rendered, "sessions": sessions, "screens": screens,
          "git_tasks":app.state::<omniterm_desktop::git_task::GitHost>().list(), "window": {
            "title": window.title().map_err(|_| "window_unavailable")?,
            "width": size.width, "height": size.height,
            "scale_factor": window.scale_factor().map_err(|_| "window_unavailable")?,
            "visible": window.is_visible().map_err(|_| "window_unavailable")?
        }}),
    )
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

fn validate_text(text: &str) -> Result<(), &'static str> {
    if text.len() > 256 || text.chars().any(char::is_control) {
        return Err("invalid_text");
    }
    Ok(())
}

fn insert_text(app: &tauri::AppHandle, text: &str) -> Result<Value, &'static str> {
    validate_text(text)?;
    protocol(&window(app)?, "Input.insertText", json!({"text": text}))?;
    Ok(json!({"text_sent": true}))
}

fn paste_text(app: &tauri::AppHandle, text: &str) -> Result<Value, &'static str> {
    validate_text(text)?;
    // Synthetic clipboard payload, scoped to the focused terminal; never reads or
    // modifies the host clipboard. This does not establish physical paste/IME.
    let value = serde_json::to_string(text).map_err(|_| "invalid_text")?;
    let expression = format!(
        r#"(() => {{
      const input = document.activeElement;
      if (!input?.matches('.terminal textarea')) return false;
      const data = new DataTransfer(); data.setData('text/plain', {value});
      input.dispatchEvent(new ClipboardEvent('paste', {{clipboardData:data, bubbles:true, cancelable:true}}));
      return true;
    }})()"#
    );
    let result = protocol(
        &window(app)?,
        "Runtime.evaluate",
        json!({"expression":expression,"returnByValue":true}),
    )?;
    if result.pointer("/result/value") != Some(&Value::Bool(true)) {
        return Err("terminal_not_focused");
    }
    Ok(json!({"paste_sent": true}))
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
    let (name, code, virtual_key, modifiers, location) = match key {
        Key::Tab => ("Tab", "Tab", 9, 0, 0),
        Key::Enter => ("Enter", "Enter", 13, 0, 0),
        Key::Escape => ("Escape", "Escape", 27, 0, 0),
        Key::Interrupt => ("c", "KeyC", 67, 2, 0),
        Key::ZoomIn => ("=", "Equal", 187, 2, 0),
        Key::ZoomPlus => ("+", "Equal", 187, 10, 0),
        Key::ZoomOut => ("-", "Minus", 189, 2, 0),
        Key::ZoomReset => ("0", "Digit0", 48, 2, 0),
        Key::NumpadAdd => ("+", "NumpadAdd", 107, 2, 3),
        Key::NumpadSubtract => ("-", "NumpadSubtract", 109, 2, 3),
        Key::ArrowLeft => ("ArrowLeft", "ArrowLeft", 37, 0, 0),
        Key::ArrowRight => ("ArrowRight", "ArrowRight", 39, 0, 0),
        Key::ArrowUp => ("ArrowUp", "ArrowUp", 38, 0, 0),
        Key::ArrowDown => ("ArrowDown", "ArrowDown", 40, 0, 0),
        Key::ResizeLeft => ("ArrowLeft", "ArrowLeft", 37, 8, 0),
        Key::ResizeRight => ("ArrowRight", "ArrowRight", 39, 8, 0),
        Key::ResizeUp => ("ArrowUp", "ArrowUp", 38, 8, 0),
        Key::ResizeDown => ("ArrowDown", "ArrowDown", 40, 8, 0),
    };
    for event in ["keyDown", "keyUp"] {
        protocol(
            &window,
            "Input.dispatchKeyEvent",
            json!({
                "type": event, "key": name, "code": code, "modifiers": modifiers,
                "location": location, "isKeypad": location == 3,
                "windowsVirtualKeyCode": virtual_key, "nativeVirtualKeyCode": virtual_key
            }),
        )?;
    }
    Ok(json!({ "key_sent": true }))
}

fn pointer(
    app: &tauri::AppHandle,
    phase: PointerPhase,
    button: PointerButton,
    x: u32,
    y: u32,
) -> Result<Value, &'static str> {
    let window = window(app)?;
    let size = window
        .inner_size()
        .map_err(|_| "window_unavailable")?
        .to_logical::<f64>(window.scale_factor().map_err(|_| "window_unavailable")?);
    validate_point(x, y, size.width, size.height)?;
    let event = match phase {
        PointerPhase::Down => "mousePressed",
        PointerPhase::Move => "mouseMoved",
        PointerPhase::Up => "mouseReleased",
    };
    let (name, held) = match button {
        PointerButton::Left => ("left", 1),
        PointerButton::Middle => ("middle", 4),
    };
    protocol(
        &window,
        "Input.dispatchMouseEvent",
        json!({
            "type": event, "x": x, "y": y, "button": name,
            "buttons": if phase == PointerPhase::Up {0} else {held}, "clickCount": 1
        }),
    )?;
    Ok(json!({"pointer_sent": true}))
}

fn validate_size(width: u32, height: u32) -> Result<(), &'static str> {
    if !(360..=1920).contains(&width) || !(300..=1080).contains(&height) {
        return Err("invalid_window_size");
    }
    Ok(())
}

// Fixed synthetic integration probes, distinct from native engine input. They
// test DOM guards/cancellation, not physical IME, OS dialogs or device capture.
fn shortcut_guards(app: &tauri::AppHandle) -> Result<Value, &'static str> {
    let result = protocol(
        &window(app)?,
        "Runtime.evaluate",
        json!({
            "expression": r#"(async () => {
          const world = document.querySelector('[data-testid="world"]');
          const before = getComputedStyle(world).transform;
          const fire = extra => {
            const event = new KeyboardEvent('keydown', {key:'+',code:'Equal',ctrlKey:true,cancelable:true,...extra});
            window.dispatchEvent(event);
            return event.defaultPrevented;
          };
          window.dispatchEvent(new CompositionEvent('compositionstart'));
          const composing = fire({});
          window.dispatchEvent(new CompositionEvent('compositionend'));
          const event_composing = fire({isComposing:true});
          const dialog = document.createElement('dialog');
          document.body.append(dialog);
          dialog.showModal();
          let modal;
          try { modal = fire({}); } finally { dialog.close(); dialog.remove(); }
          await new Promise(requestAnimationFrame);
          return {synthetic:true, composing, event_composing, modal,
            unchanged: getComputedStyle(world).transform === before};
        })()"#,
            "awaitPromise": true, "returnByValue": true
        }),
    )?;
    result
        .pointer("/result/value")
        .cloned()
        .ok_or("observation_failed")
}

fn cancel_pan(app: &tauri::AppHandle, lost_capture: bool) -> Result<Value, &'static str> {
    let expression = if lost_capture {
        r#"(() => { const viewport = document.querySelector('.board-viewport'); viewport.dispatchEvent(new PointerEvent('lostpointercapture', {pointerId:globalThis.__omniterm_test_events?.pointer_id ?? 0})); return {synthetic:true}; })()"#
    } else {
        r#"(() => { const viewport = document.querySelector('.board-viewport'); viewport.dispatchEvent(new PointerEvent('pointercancel', {pointerId:globalThis.__omniterm_test_events?.pointer_id ?? 0})); return {synthetic:true}; })()"#
    };
    protocol(
        &window(app)?,
        "Runtime.evaluate",
        json!({"expression":expression, "returnByValue":true}),
    )?;
    Ok(json!({"synthetic":true}))
}

// Fixed competing-client IPC probe for stale gesture reconciliation. It invokes
// the real authorized command; it cannot choose a command, ID, script or rect.
fn advance_layout(app: &tauri::AppHandle) -> Result<Value, &'static str> {
    let result = protocol(
        &window(app)?,
        "Runtime.evaluate",
        json!({
            "expression": r#"(async () => {
          const invoke = window.__TAURI_INTERNALS__.invoke;
          const snapshot = await invoke('get_layout_snapshot');
          const block = snapshot.blocks.find(block => block.id === '2');
          return await invoke('place_block', new TextEncoder().encode(JSON.stringify({api_version:1,
            expected_revision:snapshot.revision, block_id:'2', rect:{...block.rect,y:1}})));
        })()"#,
            "awaitPromise":true, "returnByValue":true
        }),
    )?;
    result
        .pointer("/result/value")
        .cloned()
        .ok_or("observation_failed")
}

fn resize(app: &tauri::AppHandle, width: u32, height: u32) -> Result<Value, &'static str> {
    validate_size(width, height)?;
    window(app)?
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(|_| "resize_failed")?;
    Ok(json!({"resized": true}))
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
    fn title_entry_is_bounded_and_does_not_allow_control_text() {
        assert!(validate_text("<img src=x>").is_ok());
        assert!(validate_text(&"é".repeat(128)).is_ok());
        assert!(validate_text(&"é".repeat(129)).is_err());
        assert!(validate_text("hidden\0value").is_err());
        assert!(validate_text("two\nlines").is_err());
    }

    #[test]
    fn camera_steering_has_bounded_sizes_and_fixed_operations() {
        let mut session =
            Session::from_hello(format!(r#"{{"hello":"{TOKEN}"}}"#).as_bytes()).unwrap();
        for command in [
            r#"{"pointer":{"phase":"move","button":"right","x":1,"y":2}}"#,
            r#"{"pointer":{"phase":"wheel","button":"left","x":1,"y":2}}"#,
            r#"{"key":{"key":"zoom_in","script":"bad"}}"#,
            r#"{"resize":{"width":360,"height":300,"window":"other"}}"#,
        ] {
            assert!(session
                .authorize(format!(r#"{{"token":"{TOKEN}","command":{command}}}"#).as_bytes())
                .is_err());
        }
        assert!(validate_size(360, 300).is_ok());
        assert!(validate_size(1920, 1080).is_ok());
        for (width, height) in [
            (359, 300),
            (360, 299),
            (1921, 300),
            (360, 1081),
            (u32::MAX, u32::MAX),
        ] {
            assert!(validate_size(width, height).is_err());
        }
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
