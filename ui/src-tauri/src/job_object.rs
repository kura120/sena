//! Windows Job Object management for Sena process tree grouping.
//!
//! All Sena subprocesses (daemon-bus and its children) are assigned to a
//! single Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. When the
//! UI process exits (or crashes), the Job Object handle closes and Windows
//! automatically terminates every process in the job.
//!
//! This ensures daemon-bus never outlives the UI and all processes appear
//! as a logical group in Task Manager.

#[cfg(target_os = "windows")]
mod inner {
    use std::sync::OnceLock;
    use tracing::{error, info, warn};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// Global singleton — one Job Object for the entire Sena process tree.
    static JOB_HANDLE: OnceLock<JobHandle> = OnceLock::new();

    /// RAII wrapper for the Win32 Job Object handle.
    /// Closing this handle triggers JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE.
    struct JobHandle(HANDLE);

    // Job handles are thread-safe opaque Win32 kernel objects.
    unsafe impl Send for JobHandle {}
    unsafe impl Sync for JobHandle {}

    impl Drop for JobHandle {
        fn drop(&mut self) {
            if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    /// Create the global Job Object with KILL_ON_JOB_CLOSE.
    ///
    /// Must be called once during app startup (before spawning daemon-bus).
    /// Subsequent calls are no-ops. The Job Object lives for the entire
    /// process lifetime — Windows kills all assigned processes when the
    /// handle is closed (i.e., when the UI process exits).
    pub fn init_job_object() -> Result<(), String> {
        JOB_HANDLE.get_or_init(|| {
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                    error!(
                        error_code = std::io::Error::last_os_error().raw_os_error(),
                        "Failed to create Windows Job Object"
                    );
                    // Return a dummy — callers will check assign_process_to_job errors
                    return JobHandle(INVALID_HANDLE_VALUE);
                }

                // Configure: kill all processes when the job handle closes
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

                let result = SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const std::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );

                if result == 0 {
                    let os_error = std::io::Error::last_os_error();
                    error!(
                        error = %os_error,
                        "Failed to set Job Object limits — process cleanup on exit may not work"
                    );
                    CloseHandle(handle);
                    return JobHandle(INVALID_HANDLE_VALUE);
                }

                info!("Windows Job Object created with KILL_ON_JOB_CLOSE");
                JobHandle(handle)
            }
        });

        let handle = JOB_HANDLE.get().map(|h| h.0).unwrap_or(INVALID_HANDLE_VALUE);
        if handle == INVALID_HANDLE_VALUE {
            Err("Failed to initialize Windows Job Object".to_string())
        } else {
            Ok(())
        }
    }

    /// Assign a child process to the global Job Object by its raw process handle.
    ///
    /// The process handle must have PROCESS_SET_QUOTA and PROCESS_TERMINATE
    /// access rights (which `Command::spawn()` provides by default).
    pub fn assign_process_to_job(process_handle: HANDLE) -> Result<(), String> {
        let job_handle = JOB_HANDLE
            .get()
            .map(|h| h.0)
            .unwrap_or(INVALID_HANDLE_VALUE);

        if job_handle == INVALID_HANDLE_VALUE {
            warn!("Job Object not initialized — skipping process assignment");
            return Err("Job Object not initialized".to_string());
        }

        let result = unsafe { AssignProcessToJobObject(job_handle, process_handle) };

        if result == 0 {
            let os_error = std::io::Error::last_os_error();
            error!(
                error = %os_error,
                "Failed to assign process to Job Object"
            );
            Err(format!("AssignProcessToJobObject failed: {}", os_error))
        } else {
            info!("Process assigned to Sena Job Object");
            Ok(())
        }
    }

    /// Assign a child process to the global Job Object by its PID.
    ///
    /// Opens the process with the minimum required access rights, assigns it
    /// to the job, then closes the handle.
    pub fn assign_pid_to_job(pid: u32) -> Result<(), String> {
        use windows_sys::Win32::System::Threading::OpenProcess;
        use windows_sys::Win32::System::Threading::{PROCESS_SET_QUOTA, PROCESS_TERMINATE};

        let process_handle = unsafe {
            OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid)
        };

        if process_handle.is_null() || process_handle == INVALID_HANDLE_VALUE {
            let os_error = std::io::Error::last_os_error();
            return Err(format!("OpenProcess({}) failed: {}", pid, os_error));
        }

        let result = assign_process_to_job(process_handle);

        // Always close the handle — the job holds its own reference
        unsafe {
            CloseHandle(process_handle);
        }

        result
    }
}

#[cfg(target_os = "windows")]
pub use inner::{assign_pid_to_job, init_job_object};

#[cfg(not(target_os = "windows"))]
pub fn init_job_object() -> Result<(), String> {
    Ok(()) // No-op on non-Windows
}

#[cfg(not(target_os = "windows"))]
pub fn assign_pid_to_job(_pid: u32) -> Result<(), String> {
    Ok(()) // No-op on non-Windows
}
