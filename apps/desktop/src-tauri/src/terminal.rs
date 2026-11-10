//! Runtime-owned local PTYs. UI unmounts never own process or reader lifetime.

#[cfg(windows)]
pub(crate) mod windows_job;

use crate::snapshot::WireU64;
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const CHUNK: usize = 4096;
const RAW_LIMIT: usize = 256 * 1024;
const MAX_SESSIONS: usize = 4;
pub(crate) const SCROLLBACK_ROWS: u16 = 200;

#[cfg(windows)]
pub(crate) fn shell_directory(path: PathBuf) -> PathBuf {
    use std::path::{Component, Prefix};
    // PowerShell 5 cmdlets mis-handle canonical verbatim provider paths. Preserve
    // the resolved path/OsStr while removing only the Windows verbatim prefix.
    let mut components = path.components();
    let root = match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:\\", char::from(drive))),
            Prefix::VerbatimUNC(server, share) => {
                let mut root = PathBuf::from(r"\\");
                root.push(server);
                root.push(share);
                root
            }
            _ => return path,
        },
        _ => return path,
    };
    root.join(
        components
            .filter(|part| !matches!(part, Component::RootDir))
            .collect::<PathBuf>(),
    )
}

pub fn validate_size(rows: u16, cols: u16) -> Result<(), &'static str> {
    if !(2..=60).contains(&rows) || !(10..=120).contains(&cols) {
        return Err("Terminal size must be 2..60 rows and 10..120 columns.");
    }
    Ok(())
}

pub fn validate_text(text: &str, paste: bool) -> Result<(), &'static str> {
    if text.is_empty()
        || text.len() > CHUNK
        || text
            .chars()
            .any(|c| c.is_control() && !(paste && matches!(c, '\r' | '\n' | '\t')))
    {
        return Err("Input must be 1..4096 UTF-8 bytes without unsupported control characters.");
    }
    Ok(())
}

pub fn encode_key(
    key: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    application: bool,
) -> Result<Vec<u8>, &'static str> {
    let mut bytes = if ctrl && key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() {
        vec![key.as_bytes()[0].to_ascii_uppercase() - b'@']
    } else {
        match key {
            "Enter" => b"\r".to_vec(),
            "Backspace" => vec![127],
            "Tab" if shift => b"\x1b[Z".to_vec(),
            "Tab" => b"\t".to_vec(),
            "Escape" => vec![27],
            "Delete" => b"\x1b[3~".to_vec(),
            "Home" => b"\x1b[H".to_vec(),
            "End" => b"\x1b[F".to_vec(),
            "PageUp" => b"\x1b[5~".to_vec(),
            "PageDown" => b"\x1b[6~".to_vec(),
            "ArrowUp" | "ArrowDown" | "ArrowRight" | "ArrowLeft" => {
                let final_byte = match key {
                    "ArrowUp" => b'A',
                    "ArrowDown" => b'B',
                    "ArrowRight" => b'C',
                    _ => b'D',
                };
                if ctrl || shift {
                    format!(
                        "\x1b[1;{}{}",
                        1 + u8::from(shift) + 4 * u8::from(ctrl),
                        char::from(final_byte)
                    )
                    .into_bytes()
                } else {
                    vec![27, if application { b'O' } else { b'[' }, final_byte]
                }
            }
            _ if alt && key.chars().count() == 1 && !key.chars().any(char::is_control) => {
                key.as_bytes().to_vec()
            }
            _ => return Err("Unsupported terminal key."),
        }
    };
    if alt {
        bytes.insert(0, 27);
    }
    Ok(bytes)
}

#[derive(Default)]
struct Replies {
    bytes: Vec<u8>,
}
impl vt100::Callbacks for Replies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        if i1.is_some() || i2.is_some() || self.bytes.len() > CHUNK - 32 {
            return;
        }
        let number = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        let reply = match (c, number) {
            ('n', 6) => {
                let (row, col) = screen.cursor_position();
                format!("\x1b[{};{}R", row + 1, col + 1).into_bytes()
            }
            ('n', 5) => b"\x1b[0n".to_vec(),
            ('c', 0) => b"\x1b[?1;2c".to_vec(),
            _ => return,
        };
        self.bytes.extend(reply);
    }
}

struct RawFrame {
    sequence: u64,
    elapsed_ms: u64,
    bytes: Vec<u8>,
}
struct TerminalState {
    parser: vt100::Parser<Replies>,
    frames: VecDeque<RawFrame>,
    raw_bytes: usize,
    evicted_bytes: u64,
    sequence: u64,
    revision: u64,
    started: Instant,
    exit_code: Option<u32>,
    wait_failed: bool,
    output_eof: bool,
    read_failed: bool,
    stopping: bool,
    reply_dropped: bool,
}

#[derive(Serialize)]
pub struct CellRun {
    column: u16,
    width: u16,
    pub text: String,
    pub fg: String,
    bg: String,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
}
#[derive(Serialize)]
pub struct ScreenSnapshot {
    session_id: WireU64,
    revision: WireU64,
    pub rows: u16,
    pub cols: u16,
    scrollback_rows: u16,
    scrollback_offset: u16,
    alternate_screen: bool,
    cursor_row: u16,
    cursor_col: u16,
    cursor_visible: bool,
    pub lines: Vec<Vec<CellRun>>,
    pub state: &'static str,
    exit_code: Option<u32>,
    raw_bytes: usize,
    pub evicted_bytes: u64,
    first_sequence: Option<WireU64>,
    next_sequence: WireU64,
    first_elapsed_ms: Option<u64>,
    output_eof: bool,
    read_failed: bool,
    reply_dropped: bool,
}
#[derive(Serialize)]
pub struct SessionSummary {
    pub session_id: WireU64,
    pub parent_session_id: Option<WireU64>,
    pub state: &'static str,
    exit_code: Option<u32>,
}

fn color(color: vt100::Color, background: bool) -> String {
    const BASIC: [&str; 16] = [
        "#0c0c0c", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5",
        "#666666", "#f14c4c", "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#ffffff",
    ];
    match color {
        vt100::Color::Default => if background { "#0c0c0c" } else { "#d4d4d4" }.into(),
        vt100::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        vt100::Color::Idx(n) if n < 16 => BASIC[usize::from(n)].into(),
        vt100::Color::Idx(n) if n >= 232 => {
            let v = 8 + 10 * (n - 232);
            format!("#{v:02x}{v:02x}{v:02x}")
        }
        vt100::Color::Idx(n) => {
            let n = n - 16;
            let component = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            format!(
                "#{:02x}{:02x}{:02x}",
                component(n / 36),
                component(n / 6 % 6),
                component(n % 6)
            )
        }
    }
}

impl TerminalState {
    fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: vt100::Parser::new_with_callbacks(
                NonZeroU16::new(rows).unwrap(),
                NonZeroU16::new(cols).unwrap(),
                usize::from(SCROLLBACK_ROWS),
                Replies::default(),
            ),
            frames: VecDeque::new(),
            raw_bytes: 0,
            evicted_bytes: 0,
            sequence: 0,
            revision: 0,
            started: Instant::now(),
            exit_code: None,
            wait_failed: false,
            output_eof: false,
            read_failed: false,
            stopping: false,
            reply_dropped: false,
        }
    }
    fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        self.frames.push_back(RawFrame {
            sequence: self.sequence,
            elapsed_ms: self
                .started
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
            bytes: bytes.to_vec(),
        });
        self.sequence = self.sequence.saturating_add(1);
        self.revision = self.revision.saturating_add(1);
        self.raw_bytes += bytes.len();
        while self.raw_bytes > RAW_LIMIT {
            let frame = self.frames.pop_front().unwrap();
            self.raw_bytes -= frame.bytes.len();
            self.evicted_bytes = self.evicted_bytes.saturating_add(frame.bytes.len() as u64);
        }
    }
    fn status(&self) -> &'static str {
        if self.wait_failed {
            "failed"
        } else if self.exit_code.is_some() {
            if self.output_eof {
                "exited"
            } else {
                "draining"
            }
        } else if self.stopping {
            "stopping"
        } else {
            "running"
        }
    }
    fn snapshot(&self, id: u64) -> ScreenSnapshot {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let mut lines = Vec::with_capacity(usize::from(rows.get()));
        for row in 0..rows.get() {
            let mut runs: Vec<CellRun> = Vec::new();
            for col in 0..cols.get() {
                let cell = screen.cell(row, col).unwrap();
                if cell.is_wide_continuation() {
                    if let Some(run) = runs.last_mut() {
                        run.width += 1;
                    }
                    continue;
                }
                let (fg, bg) = if cell.inverse() {
                    (color(cell.bgcolor(), true), color(cell.fgcolor(), false))
                } else {
                    (color(cell.fgcolor(), false), color(cell.bgcolor(), true))
                };
                let text = if cell.contents().is_empty() {
                    " "
                } else {
                    cell.contents()
                };
                if let Some(last) = runs.last_mut().filter(|r| {
                    r.fg == fg
                        && r.bg == bg
                        && r.bold == cell.bold()
                        && r.dim == cell.dim()
                        && r.italic == cell.italic()
                        && r.underline == cell.underline()
                }) {
                    last.width += 1;
                    last.text.push_str(text);
                } else {
                    runs.push(CellRun {
                        column: col,
                        width: 1,
                        text: text.into(),
                        fg,
                        bg,
                        bold: cell.bold(),
                        dim: cell.dim(),
                        italic: cell.italic(),
                        underline: cell.underline(),
                    });
                }
            }
            lines.push(runs);
        }
        let (cursor_row, cursor_col) = screen.cursor_position();
        ScreenSnapshot {
            session_id: id.into(),
            revision: self.revision.into(),
            rows: rows.get(),
            cols: cols.get(),
            cursor_row,
            cursor_col,
            cursor_visible: !screen.hide_cursor() && screen.scrollback() == 0,
            scrollback_rows: screen.scrollback_len().min(usize::from(SCROLLBACK_ROWS)) as u16,
            scrollback_offset: screen.scrollback().min(usize::from(SCROLLBACK_ROWS)) as u16,
            alternate_screen: screen.alternate_screen(),
            lines,
            state: self.status(),
            exit_code: self.exit_code,
            raw_bytes: self.raw_bytes,
            evicted_bytes: self.evicted_bytes,
            first_sequence: self.frames.front().map(|f| f.sequence.into()),
            next_sequence: self.sequence.into(),
            first_elapsed_ms: self.frames.front().map(|f| f.elapsed_ms),
            output_eof: self.output_eof,
            read_failed: self.read_failed,
            reply_dropped: self.reply_dropped,
        }
    }

    fn snapshot_at(&mut self, id: u64, offset: u16) -> ScreenSnapshot {
        // The caller holds the Session state lock; no parser/input work can
        // observe this presentation offset. Never copy the entire history.
        let original = self.parser.screen().scrollback();
        self.parser.screen_mut().set_scrollback(usize::from(offset));
        let snapshot = self.snapshot(id);
        self.parser.screen_mut().set_scrollback(original);
        snapshot
    }
}

struct Control {
    master: Option<Box<dyn MasterPty + Send>>,
    input: Option<mpsc::SyncSender<Vec<u8>>>,
    #[cfg(windows)]
    job: Option<windows_job::ProcessJob>,
}
struct Session {
    parent_session_id: Option<u64>,
    state: Arc<Mutex<TerminalState>>,
    control: Arc<Mutex<Control>>,
    #[cfg(windows)]
    open_access: Option<Arc<crate::open_call::windows::Access>>,
}
#[derive(Default)]
struct Sessions {
    entries: BTreeMap<u64, Arc<Session>>,
    next_id: u64,
    closing: bool,
}
pub struct TerminalHost {
    sessions: Mutex<Sessions>,
    cwd: PathBuf,
}

impl TerminalHost {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            sessions: Mutex::new(Sessions {
                next_id: 1,
                ..Sessions::default()
            }),
            cwd,
        }
    }
    fn get(&self, id: u64) -> Result<Arc<Session>, &'static str> {
        self.sessions
            .lock()
            .map_err(|_| "Terminal runtime unavailable.")?
            .entries
            .get(&id)
            .cloned()
            .ok_or("That Session does not exist.")
    }
    pub fn list(&self) -> Vec<SessionSummary> {
        self.sessions
            .lock()
            .unwrap()
            .entries
            .iter()
            .map(|(id, s)| {
                let state = s.state.lock().unwrap();
                SessionSummary {
                    session_id: (*id).into(),
                    parent_session_id: s.parent_session_id.map(Into::into),
                    state: state.status(),
                    exit_code: state.exit_code,
                }
            })
            .collect()
    }
    pub fn screen(
        &self,
        id: u64,
        after: Option<WireU64>,
    ) -> Result<Option<ScreenSnapshot>, &'static str> {
        self.screen_at(id, after, 0)
    }
    pub fn screen_at(
        &self,
        id: u64,
        after: Option<WireU64>,
        offset: u16,
    ) -> Result<Option<ScreenSnapshot>, &'static str> {
        if offset > SCROLLBACK_ROWS {
            return Err("History offset must be 0..200 rows.");
        }
        let session = self.get(id)?;
        let mut state = session.state.lock().unwrap();
        Ok(
            if offset == 0 && after.is_some_and(|v| u64::from(v) == state.revision) {
                None
            } else {
                Some(state.snapshot_at(id, offset))
            },
        )
    }
    pub fn input(&self, id: u64, text: &str, paste: bool) -> Result<(), &'static str> {
        validate_text(text, paste)?;
        let session = self.get(id)?;
        let bracketed = session
            .state
            .lock()
            .unwrap()
            .parser
            .screen()
            .bracketed_paste();
        let bytes = if paste && bracketed {
            format!("\x1b[200~{text}\x1b[201~").into_bytes()
        } else {
            text.as_bytes().to_vec()
        };
        Self::send(&session, bytes)
    }
    pub fn key(
        &self,
        id: u64,
        key: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
    ) -> Result<(), &'static str> {
        let session = self.get(id)?;
        let application = session
            .state
            .lock()
            .unwrap()
            .parser
            .screen()
            .application_cursor();
        Self::send(&session, encode_key(key, ctrl, alt, shift, application)?)
    }
    fn send(session: &Session, bytes: Vec<u8>) -> Result<(), &'static str> {
        if session.state.lock().unwrap().status() != "running" {
            return Err("Session is not running.");
        }
        session
            .control
            .lock()
            .unwrap()
            .input
            .as_ref()
            .ok_or("Session input closed.")?
            .try_send(bytes)
            .map_err(|_| "Terminal input queue is full or closed; input was not accepted.")
    }
    pub fn resize(&self, id: u64, rows: u16, cols: u16) -> Result<(), &'static str> {
        validate_size(rows, cols)?;
        let session = self.get(id)?;
        let control = session.control.lock().unwrap();
        let mut state = session.state.lock().unwrap();
        if state.parser.screen().size()
            == (
                NonZeroU16::new(rows).unwrap(),
                NonZeroU16::new(cols).unwrap(),
            )
        {
            return Ok(());
        }
        control
            .master
            .as_ref()
            .ok_or("Session has exited.")?
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|_| "Could not resize the PTY.")?;
        state.parser.set_size(
            NonZeroU16::new(rows).unwrap(),
            NonZeroU16::new(cols).unwrap(),
        );
        state.revision = state.revision.saturating_add(1);
        Ok(())
    }
    pub fn terminate(&self, id: u64) -> Result<(), &'static str> {
        let session = self.get(id)?;
        #[cfg(windows)]
        if let Some(access) = &session.open_access {
            access.revoke();
        }
        {
            let mut state = session.state.lock().unwrap();
            if state.exit_code.is_some() {
                return Ok(());
            }
            state.stopping = true;
            state.revision = state.revision.saturating_add(1);
        }
        #[cfg(windows)]
        {
            session
                .control
                .lock()
                .unwrap()
                .job
                .as_ref()
                .ok_or("Session is closing.")?
                .terminate()
                .map_err(|_| "Could not terminate the owned Session job.")?;
        }
        Ok(())
    }
    pub fn has_live(&self) -> bool {
        self.list().iter().any(|s| s.state != "exited")
    }
    pub fn shutdown(&self) -> Result<(), &'static str> {
        let ids = {
            let mut sessions = self.sessions.lock().unwrap();
            sessions.closing = true;
            sessions.entries.keys().copied().collect::<Vec<_>>()
        };
        for id in ids {
            self.terminate(id)?;
        }
        let deadline = Instant::now() + Duration::from_secs(4);
        while self.has_live() {
            if Instant::now() >= deadline {
                return Err("Owned Sessions have not confirmed exit yet; retry quit.");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    pub fn start(&self) -> Result<u64, &'static str> {
        Err("The first terminal pilot is Windows-only.")
    }
    #[cfg(windows)]
    pub fn start(&self) -> Result<u64, &'static str> {
        self.start_inner(None, &self.cwd, None)
    }
    pub(crate) fn start_with_helper(&self, app: tauri::AppHandle) -> Result<u64, &'static str> {
        #[cfg(windows)]
        {
            self.start_inner(Some(app), &self.cwd, None)
        }
        #[cfg(not(windows))]
        {
            let _ = app;
            self.start()
        }
    }
    #[cfg(windows)]
    pub(crate) fn start_child_with_helper(
        &self,
        app: tauri::AppHandle,
        parent: u64,
        source: &crate::open_call::windows::Root,
    ) -> Result<u64, &'static str> {
        source.verify()?;
        self.start_inner(Some(app), source.path(), Some(parent))
    }
    #[cfg(windows)]
    pub(crate) fn with_open_access<T>(
        &self,
        id: u64,
        pid: u32,
        action: impl FnOnce() -> Result<T, &'static str>,
    ) -> Result<T, &'static str> {
        let session = self.get(id)?;
        let access = session
            .open_access
            .as_ref()
            .ok_or("Session has no text-open authority.")?;
        access.while_active(|| {
            let control = session
                .control
                .lock()
                .map_err(|_| "Session control unavailable.")?;
            let allowed = control.job.as_ref().is_some_and(|job| job.contains(pid));
            drop(control);
            if !allowed {
                return Err("Open caller is outside the authorized Session.");
            }
            action()
        })
    }
    #[cfg(windows)]
    fn start_inner(
        &self,
        app: Option<tauri::AppHandle>,
        cwd: &Path,
        parent_session_id: Option<u64>,
    ) -> Result<u64, &'static str> {
        let mut sessions = self.sessions.lock().unwrap();
        if sessions.closing {
            return Err("Terminal runtime is closing.");
        }
        if let Some(parent) = parent_session_id {
            let parent = sessions
                .entries
                .get(&parent)
                .ok_or("Parent Session does not exist.")?;
            if parent.state.lock().unwrap().status() != "running" {
                return Err("Parent Session is no longer running.");
            }
        }
        if sessions.entries.len() >= MAX_SESSIONS {
            return Err("This pilot retains at most four Sessions; restart after stopping them to clear records.");
        }
        let next = sessions
            .next_id
            .checked_add(1)
            .ok_or("Session ID limit reached.")?;
        let cwd = cwd
            .canonicalize()
            .map_err(|_| "Shell working directory is unavailable.")?;
        let system =
            std::env::var_os("SystemRoot").ok_or("Windows system directory is unavailable.")?;
        let shell = PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        if !shell.is_file() || !cwd.is_dir() {
            return Err("The system PowerShell executable or working directory is unavailable.");
        }
        let opening = if let Some(app) = app {
            let helper = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.join("omni.exe")))
                .filter(|p| p.is_file())
                .ok_or("Build the sibling omni helper before starting a terminal.")?;
            Some((crate::open_call::windows::Server::bind(&cwd)?, app, helper))
        } else {
            None
        };
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|_| "Could not create the native PTY.")?;
        let mut command = CommandBuilder::new(shell);
        command.args(["-NoLogo", "-NoProfile"]);
        command.cwd(shell_directory(cwd));
        command.env_clear();
        for key in [
            "SystemRoot",
            "WINDIR",
            "USERPROFILE",
            "HOMEDRIVE",
            "HOMEPATH",
            "TEMP",
            "TMP",
            "PATH",
            "PATHEXT",
            "COMSPEC",
            "APPDATA",
            "LOCALAPPDATA",
            "PROGRAMFILES",
            "PROGRAMFILES(X86)",
            "PROGRAMDATA",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command.env("TERM", "xterm-256color");
        if let Some((server, _, helper)) = &opening {
            let mut paths = vec![helper.parent().unwrap().to_owned()];
            if let Some(path) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&path));
            }
            command.env(
                "PATH",
                std::env::join_paths(paths)
                    .map_err(|_| "Could not configure Session helper PATH.")?,
            );
            command.env("OMNITERM_OPEN_PIPE", &server.endpoint);
            command.env("OMNITERM_HOST_PID", std::process::id().to_string());
        }
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|_| "Could not open PTY output.")?;
        let mut writer = pair
            .master
            .take_writer()
            .map_err(|_| "Could not open PTY input.")?;
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|_| "Could not start the system shell.")?;
        let job = match child
            .as_raw_handle()
            .ok_or(())
            .and_then(|handle| windows_job::ProcessJob::attach(handle).map_err(|_| ()))
        {
            Ok(job) => job,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Could not assign shell cleanup ownership; shell was stopped.");
            }
        };
        drop(pair.slave);
        let state = Arc::new(Mutex::new(TerminalState::new(24, 80)));
        let (input_tx, input_rx) = mpsc::sync_channel::<Vec<u8>>(16);
        let control = Arc::new(Mutex::new(Control {
            master: Some(pair.master),
            input: Some(input_tx.clone()),
            job: Some(job),
        }));
        let writer_state = state.clone();
        std::thread::spawn(move || {
            while let Ok(bytes) = input_rx.recv() {
                if writer
                    .write_all(&bytes)
                    .and_then(|()| writer.flush())
                    .is_err()
                {
                    let mut state = writer_state.lock().unwrap();
                    state.read_failed = true;
                    state.revision = state.revision.saturating_add(1);
                    break;
                }
            }
        });
        let (output_tx, output_rx) = mpsc::sync_channel::<Vec<u8>>(16);
        let reader_state = state.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; CHUNK];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if output_tx.send(buffer[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        if e.kind() != std::io::ErrorKind::BrokenPipe {
                            reader_state.lock().unwrap().read_failed = true;
                        }
                        break;
                    }
                }
            }
        });
        let parser_state = state.clone();
        std::thread::spawn(move || {
            while let Ok(bytes) = output_rx.recv() {
                let replies = {
                    let mut state = parser_state.lock().unwrap();
                    state.process(&bytes);
                    std::mem::take(&mut state.parser.callbacks_mut().bytes)
                };
                if !replies.is_empty() && input_tx.try_send(replies).is_err() {
                    parser_state.lock().unwrap().reply_dropped = true;
                }
            }
            let mut state = parser_state.lock().unwrap();
            state.output_eof = true;
            state.revision = state.revision.saturating_add(1);
        });
        let waiter_state = state.clone();
        let waiter_control = control.clone();
        let open_access = opening.as_ref().map(|(server, _, _)| server.access.clone());
        let waiter_access = open_access.clone();
        std::thread::spawn(move || {
            {
                let status = child.wait();
                if let Some(access) = waiter_access {
                    access.revoke();
                }
                let mut state = waiter_state.lock().unwrap();
                match status {
                    Ok(status) => state.exit_code = Some(status.exit_code()),
                    Err(_) => state.wait_failed = true,
                };
                state.revision = state.revision.saturating_add(1);
            }
            let (job, master) = {
                let mut control = waiter_control.lock().unwrap();
                control.input.take();
                (control.job.take(), control.master.take())
            };
            drop(job);
            drop(master); // Reader/parser remain alive to drain final PTY output.
        });
        let id = sessions.next_id;
        sessions.next_id = next;
        sessions.entries.insert(
            id,
            Arc::new(Session {
                parent_session_id,
                state,
                control,
                open_access,
            }),
        );
        if let Some((server, app, _)) = opening {
            server.start(app, id);
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_snapshots_are_view_local_and_preserve_live_processing() {
        let mut state = TerminalState::new(3, 10);
        state.process(b"zero\r\none\r\ntwo\r\nthree\r\nfour");
        let live = state.snapshot(1);
        let revision = state.revision;
        let history = state.snapshot_at(1, 2);
        let text = |snapshot: &ScreenSnapshot| {
            snapshot
                .lines
                .iter()
                .map(|line| {
                    line.iter()
                        .map(|r| r.text.as_str())
                        .collect::<String>()
                        .trim_end()
                        .to_string()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(text(&live), ["two", "three", "four"]);
        assert_eq!(text(&history), ["zero", "one", "two"]);
        assert_eq!(history.scrollback_rows, 2);
        assert_eq!(history.scrollback_offset, 2);
        assert!(!history.cursor_visible);
        assert_eq!(state.revision, revision);
        assert_eq!(state.parser.screen().scrollback(), 0);
        assert_eq!(text(&state.snapshot(1)), text(&live));
        state.process(b"\r\nfive");
        assert_eq!(text(&state.snapshot(1)), ["three", "four", "five"]);
    }

    #[test]
    fn history_is_bounded_and_alternate_screen_is_independent() {
        let mut state = TerminalState::new(3, 20);
        for i in 0..300 {
            state.process(format!("LINE_{i:03}\r\n").as_bytes());
        }
        let history = state.snapshot_at(1, 200);
        assert_eq!(history.scrollback_rows, 200);
        assert_eq!(history.scrollback_offset, 200);
        assert!(!history
            .lines
            .iter()
            .flatten()
            .any(|r| r.text.contains("LINE_000")));
        state.process(b"\x1b[?1049hALT");
        let alternate = state.snapshot_at(1, 200);
        assert!(alternate.alternate_screen);
        assert_eq!(alternate.scrollback_rows, 0);
        assert_eq!(alternate.scrollback_offset, 0);
        assert!(alternate.lines[0].iter().any(|r| r.text.contains("ALT")));
        state.process(b"\x1b[?1049l");
        assert_eq!(state.snapshot_at(1, 200).scrollback_rows, 200);
        assert_eq!(state.parser.screen().scrollback(), 0);
        assert_eq!(
            TerminalHost::new(PathBuf::from("."))
                .screen_at(99, None, 201)
                .err(),
            Some("History offset must be 0..200 rows.")
        );
    }

    #[cfg(windows)]
    #[test]
    fn canonical_shell_directory_preserves_disk_and_unc_components() {
        assert_eq!(
            shell_directory(PathBuf::from(r"\\?\C:\a b\é")),
            PathBuf::from(r"C:\a b\é")
        );
        assert_eq!(
            shell_directory(PathBuf::from(r"\\?\UNC\server\share\a b")),
            PathBuf::from(r"\\server\share\a b")
        );
    }

    #[test]
    fn split_unicode_controls_and_inert_osc_produce_cells_in_rust() {
        let mut state = TerminalState::new(4, 20);
        for byte in "é界e\u{301}\x1b[31mRED\x1b[0m".as_bytes() {
            state.process(&[*byte]);
        }
        let snapshot = state.snapshot(7);
        assert!(snapshot.lines[0]
            .iter()
            .any(|run| run.fg == "#cd3131" && run.text.contains("RED")));
        assert!(state.parser.screen().contents().contains("é界e\u{301}RED"));
        state.process(b"\x1b]52;c;not-authority\x07\x1b]2;not-a-window-title\x07");
        assert!(!state.parser.screen().contents().contains("not-authority"));
        assert!(encode_key("ArrowUp", false, false, false, true)
            .unwrap()
            .starts_with(b"\x1bO"));
        assert_eq!(encode_key("c", true, false, false, false).unwrap(), b"\x03");
    }

    #[test]
    fn output_capture_is_bounded_sequenced_and_eviction_visible() {
        let mut state = TerminalState::new(4, 20);
        for _ in 0..80 {
            state.process(&vec![b'x'; CHUNK]);
        }
        assert!(state.raw_bytes <= RAW_LIMIT);
        assert_eq!(state.raw_bytes + state.evicted_bytes as usize, 80 * CHUNK);
        assert!(state.frames.front().unwrap().sequence > 0);
        assert!(state
            .frames
            .iter()
            .zip(state.frames.iter().skip(1))
            .all(|(a, b)| a.sequence + 1 == b.sequence && a.elapsed_ms <= b.elapsed_ms));
        assert_eq!(state.frames.back().unwrap().bytes.len(), CHUNK);
        assert!(state.snapshot(1).evicted_bytes > 0);
        state.process(b"\x1b]52;c;");
        for _ in 0..512 {
            state.process(&[b'a'; CHUNK]);
        }
        state.process(b"\x07\r\nAFTER");
        assert!(state.parser.screen().contents().contains("AFTER"));
    }

    #[test]
    fn input_and_size_validation_never_uses_ui_escape_output() {
        assert!(validate_size(2, 10).is_ok());
        assert!(validate_size(60, 120).is_ok());
        for size in [(0, 80), (1, 80), (61, 80), (24, 0), (24, 9), (24, 121)] {
            assert!(validate_size(size.0, size.1).is_err());
        }
        assert!(validate_text("é界", false).is_ok());
        assert!(validate_text("a\n\tb", true).is_ok());
        for text in ["\x1b[31m", "\0", "\x03"] {
            assert!(validate_text(text, true).is_err());
        }
        assert!(validate_text("line\n", false).is_err());
        assert!(validate_text(&"x".repeat(CHUNK + 1), false).is_err());
        assert!(encode_key("host_command", false, false, false, false).is_err());
    }
}
