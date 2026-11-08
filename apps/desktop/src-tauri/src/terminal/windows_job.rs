use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

pub(super) struct ProcessJob(OwnedHandle);

impl ProcessJob {
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
