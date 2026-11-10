//! One Session-scoped, bounded, inert text-open operation.

#[cfg(windows)]
pub(crate) mod windows;

use crate::snapshot::WireU64;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

pub const MAX_TEXT: usize = 1024 * 1024;
pub const MAX_REQUEST: usize = 16 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    api_version: u32,
    op: Operation,
    cwd: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    path: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Open,
    GitStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextDocument {
    pub source_session_id: WireU64,
    pub path: String,
    pub bytes: usize,
    pub text: String,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Reply {
    Ok {
        block_id: WireU64,
    },
    Git {
        block_id: WireU64,
        task: Box<crate::git_task::TaskSnapshot>,
    },
    Error {
        error: String,
    },
}

fn decode(bytes: &[u8]) -> Result<Request, &'static str> {
    if bytes.len() > MAX_REQUEST {
        return Err("Open request exceeds 16 KiB.");
    }
    let request: Request = serde_json::from_slice(bytes).map_err(|_| "Invalid open request.")?;
    if request.api_version != 1 {
        return Err("Unsupported open API version.");
    }
    if request.cwd.len() > 4096 || request.path.len() > 4096 {
        return Err("Open path exceeds 4096 UTF-8 bytes.");
    }
    match request.op {
        Operation::Open if request.path.is_empty() => {
            return Err("Open needs a relative file path.")
        }
        Operation::GitStatus if !request.path.is_empty() => {
            return Err("Git status takes no path or arguments.")
        }
        _ => {}
    }
    Ok(request)
}

fn relative_path(path: &Path) -> Result<(), &'static str> {
    let text = path.to_str().ok_or("File names must be valid Unicode.")?;
    if text.is_empty()
        || text.contains(':')
        || text.chars().any(char::is_control)
        || path
            .components()
            .any(|c| !matches!(c, Component::CurDir | Component::Normal(_)))
    {
        return Err("Use a relative file path without parent traversal, streams or device syntax.");
    }
    Ok(())
}

fn text_bytes(bytes: &[u8]) -> Result<String, &'static str> {
    if bytes.len() > MAX_TEXT {
        return Err("Text exceeds the 1 MiB limit; nothing opened.");
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "Only UTF-8 plain text is supported.")?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
    {
        return Err("Binary or unsupported control characters in text.");
    }
    Ok(text.to_owned())
}

pub fn helper_main() -> Result<(), String> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let (op, path) = if args.len() == 1 && args[0] == "git-status" {
        (Operation::GitStatus, "")
    } else if args.len() == 2 && args[0] == "open" {
        let path = args[1]
            .to_str()
            .ok_or("File names must be valid Unicode.")?;
        relative_path(Path::new(path))?;
        (Operation::Open, path)
    } else {
        return Err("Usage: omni open relative-file | omni git-status".into());
    };
    let cwd = std::env::current_dir()
        .and_then(|p| p.canonicalize())
        .map_err(|_| "Could not resolve helper cwd.")?;
    let request = Request {
        api_version: 1,
        op,
        cwd: cwd.to_str().ok_or("Cwd must be valid Unicode.")?.into(),
        path: path.into(),
    };
    #[cfg(windows)]
    {
        match windows::client(&request)? {
            Reply::Ok { block_id } => {
                println!("Opened text block #{}", u64::from(block_id));
                Ok(())
            }
            Reply::Error { error } => Err(error),
            Reply::Git { block_id, task } => {
                use std::io::Write;
                for frame in &task.frames {
                    if frame.stream == "stdout" {
                        std::io::stdout().write_all(&frame.bytes)
                    } else {
                        std::io::stderr().write_all(&frame.bytes)
                    }
                    .map_err(|_| "Could not forward Git output.")?;
                }
                println!(
                    "Git Task #{}: {} (exit {:?}); block #{}",
                    u64::from(task.task_id),
                    task.state,
                    task.exit_code,
                    u64::from(block_id)
                );
                if task.dropped_bytes > 0 {
                    eprintln!(
                        "RAW GAP: {} bytes omitted by capture limits.",
                        task.dropped_bytes
                    );
                }
                println!("{}", task.interpretation);
                if task.state == "succeeded" {
                    Ok(())
                } else {
                    Err(task.error.unwrap_or_else(|| {
                        "Git failed; inspect raw output and exit outcome.".into()
                    }))
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = request;
        Err("The text-open pilot is Windows-only.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn request_is_strict_bounded_and_has_no_claimed_session_authority() {
        assert!(
            decode(br#"{"api_version":1,"op":"open","cwd":"C:\\root","path":"report.txt"}"#)
                .is_ok()
        );
        for data in [
            br#"{"api_version":1,"op":"execute","cwd":"x","path":"a"}"#.as_slice(),
            br#"{"api_version":1,"op":"open","cwd":"x","path":"a","session_id":"1"}"#,
            br#"{"api_version":2,"op":"open","cwd":"x","path":"a"}"#,
        ] {
            assert!(decode(data).is_err());
        }
        assert!(decode(&vec![b' '; MAX_REQUEST + 1]).is_err());
    }

    #[test]
    fn paths_reject_traversal_devices_streams_and_control_names() {
        for path in [
            "../secret",
            "a/../b",
            "C:\\a",
            "\\\\server\\a",
            "a:stream",
            "a\0b",
            "",
        ] {
            assert!(relative_path(Path::new(path)).is_err(), "{path:?}");
        }
        assert!(relative_path(Path::new("./a b/é.txt")).is_ok());
    }

    #[test]
    fn text_is_inert_strict_and_never_silently_truncated() {
        assert_eq!(
            text_bytes(b"\xef\xbb\xbfhello\r\n<img src=x>\t").unwrap(),
            "hello\r\n<img src=x>\t"
        );
        assert!(text_bytes(&vec![b'x'; MAX_TEXT]).is_ok());
        for bytes in [
            &[0xff][..],
            b"bad\0value",
            b"bad\x1b[2J",
            &vec![b'x'; MAX_TEXT + 1],
        ] {
            assert!(text_bytes(bytes).is_err());
        }
    }
}
