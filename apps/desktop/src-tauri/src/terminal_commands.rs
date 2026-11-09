use crate::commands::{decode, CommandError, WorkspaceHost};
use crate::snapshot::{LayoutSnapshotV1, WireU64};
use crate::terminal::{ScreenSnapshot, SessionSummary, TerminalHost};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{ipc::InvokeBody, Manager};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    api_version: u32,
    command: Operation,
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Start {
        expected_revision: WireU64,
    },
    Reopen {
        session_id: WireU64,
        expected_revision: WireU64,
    },
    List,
    Screen {
        session_id: WireU64,
        after: Option<WireU64>,
    },
    Text {
        session_id: WireU64,
        text: String,
        paste: bool,
    },
    Key {
        session_id: WireU64,
        key: String,
        ctrl: bool,
        alt: bool,
        shift: bool,
    },
    Resize {
        session_id: WireU64,
        rows: u16,
        cols: u16,
    },
    Terminate {
        session_id: WireU64,
    },
    Quit,
}
#[derive(Default, Serialize)]
pub struct Success {
    #[serde(skip_serializing_if = "Option::is_none")]
    layout: Option<LayoutSnapshotV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    block_id: Option<WireU64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<WireU64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sessions: Option<Vec<SessionSummary>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    screen: Option<ScreenSnapshot>,
}
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Reply {
    Ok(Box<Success>),
    Error { error: CommandError },
}
fn runtime_error(message: &'static str) -> CommandError {
    CommandError::new("terminal_error", message)
}

pub fn dispatch(app: &tauri::AppHandle, body: &InvokeBody) -> Reply {
    let result = (|| {
        let request: Request = decode(body)?;
        if request.api_version != 1 {
            return Err(CommandError::new(
                "unsupported_version",
                "Unsupported terminal API version.",
            ));
        }
        let runtime = app.state::<TerminalHost>();
        let mut response = Success::default();
        match request.command {
            Operation::Start { expected_revision } => {
                let workspace = app.state::<Mutex<WorkspaceHost>>();
                let mut workspace = workspace
                    .lock()
                    .map_err(|_| runtime_error("Workspace unavailable."))?;
                workspace.check_content_room(expected_revision)?;
                let id = runtime
                    .start_with_helper(app.clone())
                    .map_err(runtime_error)?;
                match workspace.attach_terminal(id, expected_revision) {
                    Ok(block) => response.block_id = Some(block),
                    Err(error) => {
                        let _ = runtime.terminate(id);
                        return Err(error);
                    }
                }
                response.session_id = Some(id.into());
                response.layout = Some(workspace.snapshot());
                response.sessions = Some(runtime.list());
            }
            Operation::Reopen {
                session_id,
                expected_revision,
            } => {
                runtime
                    .screen(session_id.into(), None)
                    .map_err(runtime_error)?;
                let workspace = app.state::<Mutex<WorkspaceHost>>();
                let mut workspace = workspace
                    .lock()
                    .map_err(|_| runtime_error("Workspace unavailable."))?;
                response.block_id =
                    Some(workspace.attach_terminal(session_id.into(), expected_revision)?);
                response.session_id = Some(session_id);
                response.layout = Some(workspace.snapshot());
                response.sessions = Some(runtime.list());
            }
            Operation::List => response.sessions = Some(runtime.list()),
            Operation::Screen { session_id, after } => {
                response.screen = runtime
                    .screen(session_id.into(), after)
                    .map_err(runtime_error)?
            }
            Operation::Text {
                session_id,
                text,
                paste,
            } => runtime
                .input(session_id.into(), &text, paste)
                .map_err(runtime_error)?,
            Operation::Key {
                session_id,
                key,
                ctrl,
                alt,
                shift,
            } => runtime
                .key(session_id.into(), &key, ctrl, alt, shift)
                .map_err(runtime_error)?,
            Operation::Resize {
                session_id,
                rows,
                cols,
            } => runtime
                .resize(session_id.into(), rows, cols)
                .map_err(runtime_error)?,
            Operation::Terminate { session_id } => {
                runtime
                    .terminate(session_id.into())
                    .map_err(runtime_error)?;
                response.sessions = Some(runtime.list());
            }
            Operation::Quit => {
                runtime.shutdown().map_err(runtime_error)?;
                app.exit(0);
            }
        }
        Ok(response)
    })();
    match result {
        Ok(response) => Reply::Ok(Box::new(response)),
        Err(error) => Reply::Error { error },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_wire_is_strict_bounded_and_uses_opaque_ids() {
        for command in [
            r#"{"op":"screen","session_id":1}"#,
            r#"{"op":"screen","session_id":"01"}"#,
            r#"{"op":"start","expected_revision":"0","program":"anything"}"#,
            r#"{"op":"resize","session_id":"1","rows":-1,"cols":80}"#,
            r#"{"op":"resize","session_id":"1","rows":24.5,"cols":80}"#,
            r#"{"op":"execute","script":"bad"}"#,
        ] {
            let body =
                InvokeBody::Raw(format!(r#"{{"api_version":1,"command":{command}}}"#).into_bytes());
            assert!(decode::<Request>(&body).is_err());
        }
        assert!(decode::<Request>(&InvokeBody::Raw(vec![b' '; 65537])).is_err());
        assert!(decode::<Request>(&InvokeBody::Raw(
            br#"{"api_version":1,"command":{"op":"list"},"extra":true}"#.to_vec()
        ))
        .is_err());
        assert!(decode::<Request>(&InvokeBody::Raw(
            br#"{"api_version":1,"command":{"op":"screen","session_id":"1"}}"#.to_vec()
        ))
        .is_ok());
    }
}
