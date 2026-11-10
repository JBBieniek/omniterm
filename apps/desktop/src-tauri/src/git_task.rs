//! Explicit, bounded Git status Tasks; placements never own execution.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_counts_overlap_and_require_complete_success() {
        let bytes = b" M tracked.txt\r\nA  staged.txt\n?? new.txt\n";
        assert_eq!(
            interpret(bytes).unwrap(),
            Counts {
                staged: 1,
                unstaged: 1,
                untracked: 1
            }
        );
        assert_eq!(
            interpret(b"AM overlap.txt\n").unwrap(),
            Counts {
                staged: 1,
                unstaged: 1,
                untracked: 0
            }
        );
        assert_eq!(interpret(b"").unwrap(), Counts::default());
    }

    #[test]
    fn malformed_conflict_duplicate_invalid_and_giant_records_never_claim_clean() {
        for bytes in [
            b"UU conflict\n".as_slice(),
            b"R  a -> b\n",
            b"?? \n",
            b"?? a\n?? a\n",
            b" M a",
            b"\n",
            b"!! ignored\n",
            b"DD conflict\n",
            b"?? bad\xff\n",
            b"?? a\0b\n",
        ] {
            assert!(interpret(bytes).is_err(), "{bytes:?}");
        }
        assert!(interpret(&[b"?? ".as_slice(), &vec![b'x'; 16384], b"\n"].concat()).is_err());
    }

    fn capture() -> Capture {
        Capture {
            captured: 0,
            sequence: 0,
            began: Instant::now(),
            snapshot: TaskSnapshot {
                task_id: 1.into(),
                source_session_id: 7.into(),
                cwd: "test".into(),
                program: None,
                argv: Vec::new(),
                revision: 0,
                state: "running".into(),
                exit_code: None,
                stdout_eof: false,
                stderr_eof: false,
                elapsed_ms: 0,
                counts: None,
                interpretation: String::new(),
                error: None,
                dropped_bytes: 0,
                frames: Vec::new(),
            },
        }
    }
    #[test]
    fn byte_splits_stream_provenance_and_outcome_gate_preserve_raw() {
        let bytes = " M é界.txt\r\nA  staged\n?? new\n".as_bytes();
        for split in 0..=bytes.len() {
            let mut c = capture();
            c.append("stdout", &bytes[..split]);
            c.append("stderr", b"warning\n");
            c.append("stdout", &bytes[split..]);
            c.snapshot.finish_counts();
            assert!(c.snapshot.counts.is_none());
            c.snapshot.state = "succeeded".into();
            c.snapshot.exit_code = Some(0);
            c.snapshot.stdout_eof = true;
            c.snapshot.finish_counts();
            assert!(c.snapshot.counts.is_none());
            c.snapshot.stderr_eof = true;
            c.snapshot.finish_counts();
            assert_eq!(
                c.snapshot.counts,
                Some(Counts {
                    staged: 1,
                    unstaged: 1,
                    untracked: 1
                })
            );
            assert!(c
                .snapshot
                .frames
                .windows(2)
                .all(|f| f[0].sequence < f[1].sequence && f[0].elapsed_ms <= f[1].elapsed_ms));
            for state in ["failed", "cancelled", "timed_out", "draining"] {
                c.snapshot.state = state.into();
                c.snapshot.finish_counts();
                assert!(c.snapshot.counts.is_none());
            }
        }
    }
    #[test]
    fn capture_and_frame_caps_keep_prefix_and_make_gaps_visible() {
        let mut c = capture();
        for _ in 0..65 {
            c.append("stdout", &[b'x'; 4096]);
        }
        assert_eq!(c.captured, MAX_CAPTURE);
        assert_eq!(c.snapshot.dropped_bytes, 4096);
        c.snapshot.state = "succeeded".into();
        c.snapshot.exit_code = Some(0);
        c.snapshot.stdout_eof = true;
        c.snapshot.stderr_eof = true;
        c.snapshot.finish_counts();
        assert!(c.snapshot.counts.is_none());
        assert!(c.snapshot.interpretation.contains("Incomplete"));
        let mut c = capture();
        for _ in 0..513 {
            c.append("stdout", b"x");
        }
        assert_eq!(c.snapshot.frames.len(), MAX_FRAMES);
        assert_eq!(c.snapshot.dropped_bytes, 1);
        assert!(c.snapshot.frames.iter().all(|f| f.bytes == b"x"));
    }
    #[test]
    fn git_wire_does_not_grant_program_cwd_or_arguments() {
        for command in [
            r#"{"op":"refresh","task_id":"1","expected_revision":"0","cwd":"outside"}"#,
            r#"{"op":"execute","program":"anything"}"#,
            r#"{"op":"read","task_id":1}"#,
            r#"{"op":"cancel","task_id":"01"}"#,
            r#"{"op":"list","argv":[]}"#,
        ] {
            let body = tauri::ipc::InvokeBody::Raw(
                format!(r#"{{"api_version":1,"command":{command}}}"#).into_bytes(),
            );
            assert!(crate::commands::decode::<Request>(&body).is_err());
        }
    }
}

use crate::snapshot::WireU64;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

const MAX_CAPTURE: usize = 256 * 1024;
const MAX_FRAMES: usize = 512;
const MAX_TASKS: usize = 8;
const DEADLINE: Duration = Duration::from_secs(30);
const ARGS: &[&str] = &[
    "--no-optional-locks",
    "-c",
    "core.fsmonitor=",
    "-c",
    "core.untrackedCache=false",
    "-c",
    "status.renames=false",
    "status",
    "--porcelain=v1",
    "--untracked-files=all",
    "--ignore-submodules=all",
];

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Counts {
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
}

fn interpret(bytes: &[u8]) -> Result<Counts, &'static str> {
    std::str::from_utf8(bytes).map_err(|_| "Invalid UTF-8 in Git output.")?;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        return Err("Unterminated Git record.");
    }
    let mut counts = Counts::default();
    let mut paths = HashSet::new();
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.len() < 4
            || line.len() > 16384
            || line[2] != b' '
            || line.iter().any(|b| b.is_ascii_control())
        {
            return Err("Malformed or oversized Git record.");
        }
        if !paths.insert(&line[3..]) {
            return Err("Duplicate Git path record.");
        }
        match (line[0], line[1]) {
            (b'?', b'?') => counts.untracked += 1,
            (b' ', b'M' | b'T' | b'D') => counts.unstaged += 1,
            (b'M' | b'T' | b'A', b' ' | b'M' | b'T' | b'D') | (b'D', b' ') => {
                counts.staged += 1;
                if line[1] != b' ' {
                    counts.unstaged += 1;
                }
            }
            _ => return Err("Unsupported or conflicted Git record; inspect raw output."),
        }
    }
    Ok(counts)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub sequence: u64,
    pub elapsed_ms: u64,
    pub stream: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub task_id: WireU64,
    pub source_session_id: WireU64,
    pub cwd: String,
    pub program: Option<String>,
    pub argv: Vec<String>,
    pub revision: u64,
    pub state: String,
    pub exit_code: Option<i32>,
    pub stdout_eof: bool,
    pub stderr_eof: bool,
    pub elapsed_ms: u64,
    pub counts: Option<Counts>,
    pub interpretation: String,
    pub error: Option<String>,
    pub dropped_bytes: u64,
    pub frames: Vec<Frame>,
}
#[derive(Debug, Serialize)]
pub struct TaskSummary {
    pub task_id: WireU64,
    pub source_session_id: WireU64,
    pub cwd: String,
    pub state: String,
}
impl TaskSnapshot {
    fn finish_counts(&mut self) {
        self.counts = None;
        self.interpretation = "Unavailable: run did not finish successfully with both EOFs.".into();
        if self.state != "succeeded"
            || self.exit_code != Some(0)
            || !self.stdout_eof
            || !self.stderr_eof
        {
            return;
        }
        if self.dropped_bytes != 0 {
            self.interpretation =
                "Incomplete: raw capture limit reached; bytes are missing.".into();
            return;
        }
        let bytes = self
            .frames
            .iter()
            .filter(|f| f.stream == "stdout")
            .flat_map(|f| f.bytes.iter().copied())
            .collect::<Vec<_>>();
        match interpret(&bytes) {
            Ok(counts) => {
                self.counts = Some(counts);
                self.interpretation = "Complete ordinary-file snapshot; submodules excluded, renames shown as delete/add.".into();
            }
            Err(error) => self.interpretation = format!("Incomplete: {error}"),
        }
    }
}

struct Capture {
    snapshot: TaskSnapshot,
    captured: usize,
    sequence: u64,
    began: Instant,
}
impl Capture {
    fn append(&mut self, stream: &str, bytes: &[u8]) {
        self.sequence += 1;
        let keep = if self.snapshot.frames.len() < MAX_FRAMES {
            bytes.len().min(MAX_CAPTURE - self.captured)
        } else {
            0
        };
        if keep > 0 {
            self.snapshot.frames.push(Frame {
                sequence: self.sequence,
                elapsed_ms: self.began.elapsed().as_millis() as u64,
                stream: stream.into(),
                bytes: bytes[..keep].into(),
            });
            self.captured += keep;
        }
        self.snapshot.dropped_bytes += (bytes.len() - keep) as u64;
        self.snapshot.revision += 1;
    }
}

struct Task {
    capture: Mutex<Capture>,
    cancel: AtomicBool,
    #[cfg(windows)]
    source: Arc<crate::open_call::windows::Root>,
}

pub struct GitHost {
    entries: Mutex<BTreeMap<u64, Arc<Task>>>,
    closing: AtomicBool,
    program: Option<PathBuf>,
}

impl Default for GitHost {
    fn default() -> Self {
        let program = std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(if cfg!(windows) { "git.exe" } else { "git" }))
                .find_map(|p| p.is_file().then(|| p.canonicalize().ok()).flatten())
        });
        Self {
            entries: Mutex::new(BTreeMap::new()),
            closing: AtomicBool::new(false),
            program,
        }
    }
}

impl GitHost {
    #[cfg(windows)]
    pub(crate) fn start(
        &self,
        source: Arc<crate::open_call::windows::Root>,
        session: u64,
    ) -> Result<u64, &'static str> {
        let mut entries = self.entries.lock().map_err(|_| "Git Tasks unavailable.")?;
        if self.closing.load(Ordering::SeqCst) {
            return Err("App is shutting down.");
        }
        if entries.len() >= MAX_TASKS {
            return Err("Eight Git runs retained; restart the app to clear them.");
        }
        let cwd = source.path().to_string_lossy().into_owned();
        if entries.values().any(|t| {
            let s = &t.capture.lock().unwrap().snapshot;
            s.cwd == cwd && live(&s.state)
        }) {
            return Err("A Git run for this directory is already active.");
        }
        let id = entries.len() as u64 + 1;
        let task = Arc::new(Task {
            source,
            cancel: AtomicBool::new(false),
            capture: Mutex::new(Capture {
                captured: 0,
                sequence: 0,
                began: Instant::now(),
                snapshot: TaskSnapshot {
                    task_id: id.into(),
                    source_session_id: session.into(),
                    cwd,
                    program: self
                        .program
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned()),
                    argv: ARGS.iter().map(|s| (*s).into()).collect(),
                    revision: 0,
                    state: "running".into(),
                    exit_code: None,
                    stdout_eof: false,
                    stderr_eof: false,
                    elapsed_ms: 0,
                    counts: None,
                    interpretation: "Pending: waiting for exit and both EOFs.".into(),
                    error: None,
                    dropped_bytes: 0,
                    frames: Vec::new(),
                },
            }),
        });
        entries.insert(id, task.clone());
        let program = self.program.clone();
        std::thread::spawn(move || execute(task, program));
        Ok(id)
    }
    fn task(&self, id: u64) -> Result<Arc<Task>, &'static str> {
        self.entries
            .lock()
            .map_err(|_| "Git Tasks unavailable.")?
            .get(&id)
            .cloned()
            .ok_or("Unknown Git Task.")
    }
    pub fn snapshot(
        &self,
        id: u64,
        after: Option<u64>,
    ) -> Result<Option<TaskSnapshot>, &'static str> {
        let task = self.task(id)?;
        let capture = task.capture.lock().map_err(|_| "Git Task unavailable.")?;
        Ok((after != Some(capture.snapshot.revision)).then(|| capture.snapshot.clone()))
    }
    pub(crate) fn completed(&self, id: u64) -> Result<Option<TaskSnapshot>, &'static str> {
        let task = self.task(id)?;
        let capture = task.capture.lock().map_err(|_| "Git Task unavailable.")?;
        Ok((!live(&capture.snapshot.state)).then(|| capture.snapshot.clone()))
    }
    pub fn list(&self) -> Vec<TaskSummary> {
        self.entries
            .lock()
            .unwrap()
            .values()
            .map(|t| {
                let c = t.capture.lock().unwrap();
                let s = &c.snapshot;
                TaskSummary {
                    task_id: s.task_id,
                    source_session_id: s.source_session_id,
                    cwd: s.cwd.clone(),
                    state: s.state.clone(),
                }
            })
            .collect()
    }
    pub fn cancel(&self, id: u64) -> Result<(), &'static str> {
        self.task(id)?.cancel.store(true, Ordering::SeqCst);
        Ok(())
    }
    #[cfg(windows)]
    fn refresh(&self, id: u64) -> Result<u64, &'static str> {
        let task = self.task(id)?;
        let session = task.capture.lock().unwrap().snapshot.source_session_id;
        task.source.verify()?;
        self.start(task.source.clone(), session.into())
    }
    pub fn has_live(&self) -> bool {
        self.entries
            .lock()
            .unwrap()
            .values()
            .any(|t| live(&t.capture.lock().unwrap().snapshot.state))
    }
    pub fn shutdown(&self) -> Result<(), &'static str> {
        self.closing.store(true, Ordering::SeqCst);
        for task in self.entries.lock().unwrap().values() {
            task.cancel.store(true, Ordering::SeqCst);
        }
        let until = Instant::now() + Duration::from_secs(4);
        while self.has_live() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        if self.has_live() {
            Err("Git cleanup is not confirmed; keep the app open.")
        } else {
            Ok(())
        }
    }
}
fn live(state: &str) -> bool {
    matches!(state, "running" | "draining")
}

fn drain(mut reader: impl Read, task: Arc<Task>, stream: &'static str) {
    let mut bytes = [0u8; 4096];
    loop {
        match reader.read(&mut bytes) {
            Ok(0) => {
                let mut c = task.capture.lock().unwrap();
                if stream == "stdout" {
                    c.snapshot.stdout_eof = true;
                } else {
                    c.snapshot.stderr_eof = true;
                }
                c.snapshot.revision += 1;
                break;
            }
            Ok(n) => task.capture.lock().unwrap().append(stream, &bytes[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                task.capture.lock().unwrap().snapshot.error =
                    Some("Pipe read failed; raw source is incomplete.".into());
                break;
            }
        }
    }
}

#[cfg(windows)]
fn execute(task: Arc<Task>, program: Option<PathBuf>) {
    use std::os::windows::{io::AsRawHandle, process::CommandExt};
    let result = (|| {
        task.source.verify()?;
        let program =
            program.ok_or("Git executable was not found in the app's absolute PATH entries.")?;
        let mut command = std::process::Command::new(program);
        command
            .args(ARGS)
            .current_dir(task.source.path())
            .env_clear();
        for key in [
            "SystemRoot",
            "WINDIR",
            "PATH",
            "TEMP",
            "TMP",
            "USERPROFILE",
            "HOMEDRIVE",
            "HOMEPATH",
            "APPDATA",
            "LOCALAPPDATA",
            "PROGRAMDATA",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "NUL")
            .env("GIT_CEILING_DIRECTORIES", task.source.git_ceiling()?)
            .env("GIT_DISCOVERY_ACROSS_FILESYSTEM", "0")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .creation_flags(0x08000000);
        let mut child = command
            .spawn()
            .map_err(|_| "Could not launch Git; no automatic retry.")?;
        let job = match crate::terminal::windows_job::ProcessJob::attach(child.as_raw_handle()) {
            Ok(job) => job,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Could not own Git process lifetime.");
            }
        };
        let out = child.stdout.take().unwrap();
        let err = child.stderr.take().unwrap();
        let a = task.clone();
        let stdout = std::thread::spawn(move || drain(out, a, "stdout"));
        let a = task.clone();
        let stderr = std::thread::spawn(move || drain(err, a, "stderr"));
        let began = Instant::now();
        let mut stopped = None;
        let status = loop {
            if stopped.is_none()
                && (task.cancel.load(Ordering::SeqCst) || began.elapsed() >= DEADLINE)
            {
                stopped = Some(if task.cancel.load(Ordering::SeqCst) {
                    "cancelled"
                } else {
                    "timed_out"
                });
                let _ = job.terminate();
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => {
                    let _ = job.terminate();
                    let _ = child.wait();
                    break Err("Git exit status unavailable.");
                }
            }
        };
        {
            let mut c = task.capture.lock().unwrap();
            c.snapshot.state = "draining".into();
            c.snapshot.revision += 1;
        }
        drop(job); // Also close any owned descendants retaining a pipe before joining readers.
        let stdout_joined = stdout.join().is_ok();
        let stderr_joined = stderr.join().is_ok();
        let joined = stdout_joined && stderr_joined;
        let mut c = task.capture.lock().unwrap();
        match status {
            Ok(status) => {
                c.snapshot.exit_code = status.code();
                c.snapshot.state = stopped
                    .unwrap_or(if status.success() {
                        "succeeded"
                    } else {
                        "failed"
                    })
                    .into();
            }
            Err(error) => {
                c.snapshot.state = "failed".into();
                c.snapshot.error = Some(error.into());
            }
        }
        if !joined || !c.snapshot.stdout_eof || !c.snapshot.stderr_eof {
            c.snapshot.state = "failed".into();
            c.snapshot.error = Some("Output drain failed; result is incomplete.".into());
        }
        c.snapshot.elapsed_ms = c.began.elapsed().as_millis() as u64;
        c.snapshot.finish_counts();
        c.snapshot.revision += 1;
        Ok::<(), &'static str>(())
    })();
    let mut c = task.capture.lock().unwrap();
    if let Err(error) = result {
        c.snapshot.state = "failed".into();
        c.snapshot.error = Some(error.into());
    }
    if result.is_err() {
        c.snapshot.elapsed_ms = c.began.elapsed().as_millis() as u64;
        c.snapshot.finish_counts();
        c.snapshot.revision += 1;
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    api_version: u32,
    command: Operation,
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    List {},
    Read {
        task_id: WireU64,
        after: Option<u64>,
    },
    Cancel {
        task_id: WireU64,
    },
    Reopen {
        task_id: WireU64,
        expected_revision: WireU64,
    },
    Refresh {
        task_id: WireU64,
        expected_revision: WireU64,
    },
}

pub(crate) fn dispatch(
    app: &tauri::AppHandle,
    body: &tauri::ipc::InvokeBody,
) -> Result<serde_json::Value, String> {
    use tauri::{Emitter, Manager};
    let request: Request = crate::commands::decode(body).map_err(|e| e.message)?;
    if request.api_version != 1 {
        return Err("Unsupported Git API version.".into());
    }
    let host = app.state::<GitHost>();
    Ok(match request.command {
        Operation::List {} => serde_json::json!({"tasks":host.list()}),
        Operation::Read { task_id, after } => {
            serde_json::json!({"task":host.snapshot(task_id.into(), after)?})
        }
        Operation::Cancel { task_id } => {
            host.cancel(task_id.into())?;
            serde_json::json!({"cancel_requested":true})
        }
        Operation::Refresh {
            task_id,
            expected_revision,
        }
        | Operation::Reopen {
            task_id,
            expected_revision,
        } => {
            let refresh = matches!(request.command, Operation::Refresh { .. });
            host.task(task_id.into())?;
            let workspace = app.state::<Mutex<crate::commands::WorkspaceHost>>();
            let mut workspace = workspace.lock().map_err(|_| "Workspace unavailable.")?;
            if refresh {
                workspace
                    .check_content_room(expected_revision)
                    .map_err(|e| e.message)?;
            }
            #[cfg(windows)]
            let id = if refresh {
                host.refresh(task_id.into())?
            } else {
                task_id.into()
            };
            #[cfg(not(windows))]
            let id = {
                let _ = refresh;
                return Err("Git Tasks are Windows-only in this pilot.".into());
            };
            let block = workspace
                .attach_git(id, expected_revision)
                .map_err(|e| e.message)?;
            let _ = app.emit("layout-changed", block);
            serde_json::json!({"layout":workspace.snapshot(),"block_id":block,"task_id":WireU64::from(id)})
        }
    })
}
