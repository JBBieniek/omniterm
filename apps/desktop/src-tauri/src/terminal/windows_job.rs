use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

pub(super) struct ProcessJob(OwnedHandle);

impl ProcessJob {
    pub fn contains(&self, pid: u32) -> bool {
        use windows::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
        use windows::Win32::System::JobObjects::IsProcessInJob;
        use windows::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE,
        };
        // SAFETY: OS-reported peer PID, held live process handle, and our owned job.
        unsafe {
            let Ok(process) = OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            ) else {
                return false;
            };
            let mut inside = windows::core::BOOL::default();
            let allowed = WaitForSingleObject(process, 0) == WAIT_TIMEOUT
                && IsProcessInJob(process, Some(HANDLE(self.0.as_raw_handle())), &mut inside)
                    .is_ok()
                && inside.as_bool();
            let _ = CloseHandle(process);
            allowed
        }
    }
    pub fn attach(process: RawHandle) -> windows::core::Result<Self> {
        // SAFETY: new job handle is uniquely owned, child handle is borrowed
        // from the live portable-pty Child, and the limit structure has its ABI size.
        unsafe {
            let handle = CreateJobObjectW(None, None)?;
            let owned = OwnedHandle::from_raw_handle(handle.0);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                std::mem::size_of_val(&limits) as u32,
            )?;
            AssignProcessToJobObject(handle, HANDLE(process))?;
            Ok(Self(owned))
        }
    }
    pub fn terminate(&self) -> windows::core::Result<()> {
        // SAFETY: the owned, non-inherited job remains live for this call.
        unsafe { TerminateJobObject(HANDLE(self.0.as_raw_handle()), 1) }
    }
}
