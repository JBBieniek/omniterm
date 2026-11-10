use super::{
    decode, relative_path, text_bytes, Reply, Request, TextDocument, MAX_REQUEST, MAX_TEXT,
};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::windows::{
    ffi::OsStringExt,
    fs::{MetadataExt, OpenOptionsExt},
    io::{AsRawHandle, FromRawHandle},
};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use windows::core::HSTRING;
use windows::Win32::Foundation::*;
use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Pipes::*;
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects, INFINITE};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

fn handle(file: &File) -> HANDLE {
    HANDLE(file.as_raw_handle())
}
fn own(value: HANDLE) -> File {
    // SAFETY: every caller passes a newly created, uniquely owned valid handle.
    unsafe { File::from_raw_handle(value.0) }
}

pub(crate) struct Access {
    active: Mutex<bool>,
    revoked: File,
}
impl Access {
    fn new() -> Result<Arc<Self>, &'static str> {
        // SAFETY: no inherited handle/security pointer; event has one owned handle.
        let event = unsafe { CreateEventW(None, true, false, None) }
            .map_err(|_| "Could not create helper revocation event.")?;
        Ok(Arc::new(Self {
            active: Mutex::new(true),
            revoked: own(event),
        }))
    }
    pub fn revoke(&self) {
        *self.active.lock().unwrap() = false;
        // SAFETY: the event remains owned throughout this call.
        let _ = unsafe { SetEvent(handle(&self.revoked)) };
    }
    pub fn while_active<T>(
        &self,
        action: impl FnOnce() -> Result<T, &'static str>,
    ) -> Result<T, &'static str> {
        let guard = self
            .active
            .lock()
            .map_err(|_| "Session access unavailable.")?;
        if !*guard {
            return Err("Session text-open access is revoked.");
        }
        action()
    }
}

// All buffers and OVERLAPPED/event storage remain alive until completion, including
// cancellation. Only pipes use this function; no blocking disk operation holds it.
fn io(
    pipe: &File,
    stop: Option<&Access>,
    timeout: u32,
    issue: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>,
) -> Result<u32, &'static str> {
    unsafe {
        let event = own(CreateEventW(None, true, false, None)
            .map_err(|_| "Could not create pipe I/O event.")?);
        let mut operation = OVERLAPPED {
            hEvent: handle(&event),
            ..Default::default()
        };
        match issue(&mut operation) {
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => return Ok(0),
            Err(e) if e.code() != ERROR_IO_PENDING.to_hresult() => {
                return Err(if e.code() == ERROR_MORE_DATA.to_hresult() {
                    "Open message exceeds its size limit."
                } else {
                    "Open pipe I/O failed."
                })
            }
            _ => {}
        }
        let mut events = vec![handle(&event)];
        if let Some(access) = stop {
            events.push(handle(&access.revoked));
        }
        let result = WaitForMultipleObjects(&events, false, timeout);
        if result != WAIT_OBJECT_0 {
            let _ = CancelIoEx(handle(pipe), Some(&operation));
            let mut transferred = 0;
            let _ = GetOverlappedResult(handle(pipe), &operation, &mut transferred, true);
            return Err(
                "Open pipe timed out or Session access was revoked; request was not retried.",
            );
        }
        let mut transferred = 0;
        GetOverlappedResult(handle(pipe), &operation, &mut transferred, false)
            .map_err(|_| "Open pipe completion failed.")?;
        Ok(transferred)
    }
}
fn receive(
    pipe: &File,
    stop: Option<&Access>,
    limit: usize,
    timeout: u32,
) -> Result<Vec<u8>, &'static str> {
    let mut bytes = vec![0; limit];
    let count = io(pipe, stop, timeout, |op| unsafe {
        ReadFile(handle(pipe), Some(&mut bytes), None, Some(op))
    })?;
    bytes.truncate(count as usize);
    Ok(bytes)
}
fn send(pipe: &File, stop: Option<&Access>, bytes: &[u8]) -> Result<(), &'static str> {
    let count = io(pipe, stop, 2000, |op| unsafe {
        WriteFile(handle(pipe), Some(bytes), None, Some(op))
    })?;
    if count as usize != bytes.len() {
        return Err("Open response incomplete; do not retry automatically.");
    }
    Ok(())
}

fn final_path(file: &File) -> Result<PathBuf, &'static str> {
    let mut name = vec![0u16; 32768];
    // SAFETY: valid owned file handle and bounded output buffer, no pointer retained.
    let count = unsafe {
        GetFinalPathNameByHandleW(handle(file), &mut name, GETFINALPATHNAMEBYHANDLE_FLAGS(0))
    } as usize;
    if count == 0 || count >= name.len() {
        return Err("Could not verify final file path.");
    }
    Ok(std::ffi::OsString::from_wide(&name[..count]).into())
}

pub(crate) struct Root {
    file: File,
    path: PathBuf,
    ceiling: PathBuf,
}
impl Root {
    pub(crate) fn git_ceiling(&self) -> Result<PathBuf, &'static str> {
        let path = crate::terminal::shell_directory(self.ceiling.clone());
        if path.to_string_lossy().contains(';') {
            return Err("Semicolons in the Git discovery ceiling are unsupported.");
        }
        Ok(path)
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn verify(&self) -> Result<(), &'static str> {
        if final_path(&self.file)? != self.path {
            return Err("Authorized directory moved; create a new run from the Session.");
        }
        Ok(())
    }
    pub(crate) fn directory(&self, cwd: &str) -> Result<Arc<Self>, &'static str> {
        self.verify()?;
        let path = Path::new(cwd);
        if cwd.chars().any(char::is_control)
            || path.components().any(|c| matches!(c, Component::ParentDir))
        {
            return Err("Invalid helper cwd.");
        }
        let relative = path
            .strip_prefix(&self.path)
            .map_err(|_| "Helper cwd is outside the authorized root.")?;
        let mut candidate = self.path.clone();
        for part in relative.components() {
            if !matches!(part, Component::Normal(_)) {
                return Err("Invalid helper cwd.");
            }
            candidate.push(part);
            if std::fs::symlink_metadata(&candidate)
                .map_err(|_| "Cwd unavailable.")?
                .file_attributes()
                & FILE_ATTRIBUTE_REPARSE_POINT.0
                != 0
            {
                return Err("Reparse-point cwd is not supported.");
            }
        }
        let mut root = Self::open(path)?;
        root.ceiling = self.ceiling.clone();
        if !final_path(&root.file)?.starts_with(final_path(&self.file)?) {
            return Err("Cwd handle is outside authorized root.");
        }
        root.verify()?;
        Ok(Arc::new(root))
    }
    pub fn open(path: &Path) -> Result<Self, &'static str> {
        let path = path
            .canonicalize()
            .map_err(|_| "Open root is unavailable.")?;
        if !matches!(path.components().next(),Some(Component::Prefix(p)) if matches!(p.kind(),Prefix::VerbatimDisk(_)|Prefix::Disk(_)))
        {
            return Err("Text-open root must be on a local disk.");
        }
        let file = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES.0)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags((FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).0)
            .open(&path)
            .map_err(|_| "Could not pin the authorized root.")?;
        if !file
            .metadata()
            .map_err(|_| "Root metadata unavailable.")?
            .is_dir()
        {
            return Err("Open root must be a directory.");
        }
        let ceiling = path.parent().unwrap_or(&path).to_owned();
        Ok(Self {
            file,
            path,
            ceiling,
        })
    }
    pub fn read(&self, request: &Request, session: u64) -> Result<TextDocument, &'static str> {
        relative_path(Path::new(&request.path))?;
        let cwd = Path::new(&request.cwd);
        if request.cwd.chars().any(char::is_control)
            || cwd.components().any(|c| matches!(c, Component::ParentDir))
        {
            return Err("Invalid helper cwd.");
        }
        let cwd_relative = cwd
            .strip_prefix(&self.path)
            .map_err(|_| "Helper cwd is outside the authorized root.")?;
        let relative = cwd_relative.join(&request.path);
        let mut candidate = self.path.clone();
        for part in relative.components() {
            match part {
                Component::CurDir => continue,
                Component::Normal(_) => candidate.push(part),
                _ => return Err("Invalid relative file path."),
            }
            let metadata = std::fs::symlink_metadata(&candidate)
                .map_err(|_| "File is missing or inaccessible.")?;
            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
                return Err("Symlinks and reparse points are not supported.");
            }
        }
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(&candidate)
            .map_err(|_| "File is busy, missing or inaccessible.")?;
        let metadata = file.metadata().map_err(|_| "File metadata unavailable.")?;
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err("Only regular text files are supported.");
        }
        let root_before = final_path(&self.file)?;
        let actual = final_path(&file)?;
        if !actual.starts_with(&root_before) || actual == root_before {
            return Err("Final file handle is outside the authorized root.");
        }
        // Deny devices and hard links; no reads occur before this handle-based scope check.
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            if GetFileType(handle(&file)) != FILE_TYPE_DISK
                || GetFileInformationByHandle(handle(&file), &mut info).is_err()
                || info.nNumberOfLinks != 1
            {
                return Err("Only singly linked local disk files are supported.");
            }
        }
        if metadata.len() > MAX_TEXT as u64 {
            return Err("Text exceeds the 1 MiB limit; nothing opened.");
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_TEXT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read text file.")?;
        let text = text_bytes(&bytes)?;
        if final_path(&self.file)? != root_before || final_path(&file)? != actual {
            return Err("File scope changed during the read; nothing opened.");
        }
        let relative = actual
            .strip_prefix(root_before)
            .map_err(|_| "File scope changed.")?;
        Ok(TextDocument {
            source_session_id: session.into(),
            path: relative
                .to_str()
                .ok_or("File name must be valid Unicode.")?
                .into(),
            bytes: bytes.len(),
            text,
        })
    }
}

pub(crate) struct Server {
    pub endpoint: String,
    pipe: File,
    pub access: Arc<Access>,
    root: Root,
}
impl Server {
    pub fn bind(root: &Path) -> Result<Self, &'static str> {
        let mut nonce = [0u8; 32];
        unsafe { BCryptGenRandom(None, &mut nonce, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }
            .ok()
            .map_err(|_| "Could not create open endpoint identity.")?;
        let endpoint = format!(
            r"\\.\pipe\omniterm-open-{}",
            nonce.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        let pipe = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(&endpoint),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                MAX_REQUEST as u32,
                MAX_REQUEST as u32,
                2000,
                None,
            )
        };
        if pipe == INVALID_HANDLE_VALUE {
            return Err("Could not create the private open endpoint.");
        }
        Ok(Self {
            endpoint,
            pipe: own(pipe),
            access: Access::new()?,
            root: Root::open(root)?,
        })
    }
    pub fn start(self, app: tauri::AppHandle, session: u64) {
        std::thread::spawn(move || {
            loop {
                if io(&self.pipe, Some(&self.access), INFINITE, |op| unsafe {
                    ConnectNamedPipe(handle(&self.pipe), Some(op))
                })
                .is_err()
                {
                    break;
                }
                let result = (|| {
                    let mut pid = 0;
                    unsafe { GetNamedPipeClientProcessId(handle(&self.pipe), &mut pid) }
                        .map_err(|_| "Could not authenticate open caller.")?;
                    let runtime = app.state::<crate::terminal::TerminalHost>();
                    let authorized = runtime.with_open_access(session, pid, || Ok(()));
                    let bytes = receive(&self.pipe, Some(&self.access), MAX_REQUEST, 2000)?;
                    authorized?; // Drain a bounded frame before a denial, without parsing it.
                    let request = decode(&bytes)?;
                    if matches!(request.op, super::Operation::Terminal) {
                        let source = self.root.directory(&request.cwd)?;
                        let (block_id, child) = runtime.with_open_access(session, pid, || {
                            let workspace = app.state::<Mutex<crate::commands::WorkspaceHost>>();
                            let mut workspace =
                                workspace.lock().map_err(|_| "Workspace unavailable.")?;
                            let revision = workspace.current_revision();
                            workspace
                                .check_content_room(revision)
                                .map_err(|e| e.message)?;
                            let child =
                                runtime.start_child_with_helper(app.clone(), session, &source)?;
                            match workspace.attach_terminal(child, revision) {
                                Ok(block) => Ok((block, child)),
                                Err(error) => {
                                    let _ = runtime.terminate(child);
                                    Err(error.message)
                                }
                            }
                        })?;
                        let _ = app.emit("layout-changed", block_id);
                        return Ok(Reply::Terminal {
                            block_id,
                            session_id: child.into(),
                        });
                    }
                    if matches!(request.op, super::Operation::GitStatus) {
                        let source = self.root.directory(&request.cwd)?;
                        let (id, task_id) = runtime.with_open_access(session, pid, || {
                            let workspace = app.state::<Mutex<crate::commands::WorkspaceHost>>();
                            let mut workspace =
                                workspace.lock().map_err(|_| "Workspace unavailable.")?;
                            let revision = workspace.current_revision();
                            workspace
                                .check_content_room(revision)
                                .map_err(|e| e.message)?;
                            let host = app.state::<crate::git_task::GitHost>();
                            let task = host.start(source, session)?;
                            let block = workspace
                                .attach_git(task, revision)
                                .map_err(|e| e.message)?;
                            Ok((block, task))
                        })?;
                        let _ = app.emit("layout-changed", id);
                        loop {
                            let task =
                                app.state::<crate::git_task::GitHost>().completed(task_id)?;
                            if let Some(task) = task {
                                return Ok(Reply::Git {
                                    block_id: id,
                                    task: Box::new(task),
                                });
                            }
                            self.access.while_active(|| Ok(()))?;
                            std::thread::sleep(Duration::from_millis(25));
                        }
                    }
                    let document = self.root.read(&request, session)?;
                    let id = runtime.with_open_access(session, pid, || {
                        app.state::<Mutex<crate::commands::WorkspaceHost>>()
                            .lock()
                            .map_err(|_| "Workspace unavailable.")?
                            .add_text(document)
                            .map_err(|error| error.message)
                    })?;
                    let _ = app.emit("layout-changed", id);
                    Ok::<Reply, &'static str>(Reply::Ok { block_id: id })
                })();
                let reply = match result {
                    Ok(reply) => reply,
                    Err(error) => Reply::Error {
                        error: error.into(),
                    },
                };
                if let Ok(bytes) = serde_json::to_vec(&reply) {
                    if send(&self.pipe, Some(&self.access), &bytes).is_ok() {
                        let _ = receive(&self.pipe, Some(&self.access), 1, 2000);
                    }
                }
                let _ = unsafe { DisconnectNamedPipe(handle(&self.pipe)) };
                // Bound sequential request rate without holding any runtime/workspace lock.
                unsafe { WaitForMultipleObjects(&[handle(&self.access.revoked)], false, 250) };
                if self.access.while_active(|| Ok(())).is_err() {
                    break;
                }
            }
        });
    }
}

pub(crate) fn client(request: &Request) -> Result<Reply, String> {
    let endpoint =
        std::env::var("OMNITERM_OPEN_PIPE").map_err(|_| "Run omni inside an Omniterm Session.")?;
    let suffix = endpoint
        .strip_prefix(r"\\.\pipe\omniterm-open-")
        .ok_or("Invalid local open endpoint.")?;
    if suffix.len() != 64 || !suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid local open endpoint.".into());
    }
    let expected = std::env::var("OMNITERM_HOST_PID")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or("Missing host identity.")?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let pipe = loop {
        let opened = unsafe {
            CreateFileW(
                &HSTRING::from(&endpoint),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        };
        match opened {
            Ok(value) => break own(value),
            Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() && Instant::now() < deadline => {
                let _ = unsafe { WaitNamedPipeW(&HSTRING::from(&endpoint), 100) };
            }
            _ => return Err("Open endpoint unavailable or revoked; nothing retried.".into()),
        }
    };
    let mut actual = 0;
    unsafe { GetNamedPipeServerProcessId(handle(&pipe), &mut actual) }
        .map_err(|_| "Could not verify open server.")?;
    if actual != expected {
        return Err("Open server identity mismatch.".into());
    }
    unsafe { SetNamedPipeHandleState(handle(&pipe), Some(&PIPE_READMODE_MESSAGE), None, None) }
        .map_err(|_| "Could not configure open protocol.")?;
    let bytes = serde_json::to_vec(request).map_err(|_| "Could not encode open request.")?;
    if bytes.len() > MAX_REQUEST {
        return Err("Open request exceeds 16 KiB.".into());
    }
    send(&pipe, None, &bytes)?;
    let (limit, timeout) = if matches!(request.op, super::Operation::GitStatus) {
        (2 * 1024 * 1024, 35000)
    } else {
        (1024, 5000)
    };
    let bytes = receive(&pipe, None, limit, timeout)?;
    let reply = serde_json::from_slice(&bytes)
        .map_err(|_| "Invalid open response; request was not retried.")?;
    send(&pipe, None, &[1])?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verified_file_read_uses_current_cwd_bounds_and_rejects_hard_links() {
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let base = temp.join(format!(
            "omniterm-open-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&base).unwrap();
        let folder = base.join("root");
        std::fs::create_dir(&folder).unwrap();
        let sub = folder.join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(folder.join("report.txt"), b"initial").unwrap();
        std::fs::write(sub.join("report.txt"), "current é界 <img src=x>").unwrap();
        std::fs::write(base.join("outside.txt"), b"outside").unwrap();
        std::fs::hard_link(base.join("outside.txt"), sub.join("linked.txt")).unwrap();
        let root = Root::open(&folder).unwrap();
        let request = |cwd: &Path, path: &str| Request {
            api_version: 1,
            op: super::super::Operation::Open,
            cwd: cwd.canonicalize().unwrap().to_str().unwrap().into(),
            path: path.into(),
        };
        let document = root.read(&request(&sub, "./report.txt"), 7).unwrap();
        assert_eq!(document.text, "current é界 <img src=x>");
        assert_eq!(document.path, r"sub\report.txt");
        assert!(root.read(&request(&base, "outside.txt"), 7).is_err());
        for name in [
            "../report.txt",
            "missing.txt",
            "linked.txt",
            "report.txt:stream",
            ".",
        ] {
            assert!(root.read(&request(&sub, name), 7).is_err(), "{name}");
        }
        std::fs::write(sub.join("large.txt"), vec![b'x'; MAX_TEXT + 1]).unwrap();
        assert!(root.read(&request(&sub, "large.txt"), 7).is_err());
        let writer = OpenOptions::new()
            .write(true)
            .open(sub.join("report.txt"))
            .unwrap();
        assert!(root.read(&request(&sub, "report.txt"), 7).is_err());
        drop(writer);
        drop(root);
        let resolved = base.canonicalize().unwrap();
        assert!(
            resolved.starts_with(temp)
                && resolved
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("omniterm-open-")
        );
        std::fs::remove_dir_all(resolved).unwrap();
    }
    #[test]
    fn revocation_linearizes_against_final_commit_and_wakes_io_event() {
        let access = Access::new().unwrap();
        assert_eq!(access.while_active(|| Ok(7)).unwrap(), 7);
        access.revoke();
        assert!(access.while_active(|| Ok(7)).is_err());
        assert_eq!(
            unsafe { WaitForMultipleObjects(&[handle(&access.revoked)], false, 0) },
            WAIT_OBJECT_0
        );
    }
}
