//! Native local Windows service inventory and controls for confirmed UI actions.

#[derive(Clone, Debug)]
pub struct ServiceRow {
    pub name: String,
    pub display_name: String,
    pub state: String,
    pub pid: u32,
}

#[derive(Clone, Copy, Debug)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
}

/// Snapshot local Win32 services, sorted by service name. Does not request control rights.
pub fn list() -> Result<Vec<ServiceRow>, String> {
    #[cfg(windows)]
    {
        windows::list()
    }
    #[cfg(not(windows))]
    {
        Err("Windows services are only supported on Windows".into())
    }
}

/// Apply an action already confirmed by the UI to a name from its live inventory.
/// Does not elevate. After opening the handle, uses one ten-second polling budget
/// across both restart phases. This is not a wall-clock limit: SCM calls can block
/// beyond it, and a late query observing the target state can still succeed.
/// Call on a worker; a polling timeout does not cancel an accepted SCM request.
pub fn control(name: &str, action: ServiceAction) -> Result<String, String> {
    if name.contains('\0') {
        return Err("Service name must not contain NUL".into());
    }
    if name.trim().is_empty() {
        return Err("Service name must not be empty".into());
    }
    crate::release::require_system_mutations()?;
    #[cfg(windows)]
    {
        windows::control(name, action)
    }
    #[cfg(not(windows))]
    {
        let _ = action;
        Err("Windows services are only supported on Windows".into())
    }
}

#[cfg(windows)]
mod windows {
    use super::{ServiceAction, ServiceRow};
    use std::mem::size_of;
    use std::ptr::null;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_MORE_DATA, ERROR_SERVICE_ALREADY_RUNNING,
        ERROR_SERVICE_NOT_ACTIVE, GetLastError,
    };
    use windows_sys::Win32::System::Services::*;

    struct Handle(SC_HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: each handle is owned exactly once and closed after its uses.
            unsafe {
                CloseServiceHandle(self.0);
            }
        }
    }

    fn error(operation: &str, code: u32) -> String {
        if code == ERROR_ACCESS_DENIED {
            format!(
                "{operation}: access denied (Windows error {code}); current account lacks permission"
            )
        } else {
            format!(
                "{operation}: {} (Windows error {code})",
                std::io::Error::from_raw_os_error(code as i32)
            )
        }
    }

    fn manager(rights: u32) -> Result<Handle, String> {
        // SAFETY: null names select the local machine and default database.
        let handle = unsafe { OpenSCManagerW(null(), null(), rights) };
        if handle.is_null() {
            Err(error("OpenSCManagerW", unsafe { GetLastError() }))
        } else {
            Ok(Handle(handle))
        }
    }

    fn state_name(state: u32) -> String {
        match state {
            SERVICE_STOPPED => "stopped".into(),
            SERVICE_START_PENDING => "start_pending".into(),
            SERVICE_STOP_PENDING => "stop_pending".into(),
            SERVICE_RUNNING => "running".into(),
            SERVICE_CONTINUE_PENDING => "continue_pending".into(),
            SERVICE_PAUSE_PENDING => "pause_pending".into(),
            SERVICE_PAUSED => "paused".into(),
            _ => format!("unknown ({state})"),
        }
    }

    // Validate returned string pointers against the still-live enumeration buffer.
    fn buffer_string(pointer: *const u16, buffer: &[usize]) -> Result<String, String> {
        let start = buffer.as_ptr() as usize;
        let end = start + std::mem::size_of_val(buffer);
        let address = pointer as usize;
        if address < start || address >= end || !address.is_multiple_of(2) {
            return Err("EnumServicesStatusExW returned an invalid string pointer".into());
        }
        // SAFETY: the buffer is aligned, initialized, live, and bounds checked.
        let units = unsafe { std::slice::from_raw_parts(pointer, (end - address) / 2) };
        let length = units
            .iter()
            .position(|unit| *unit == 0)
            .ok_or_else(|| "EnumServicesStatusExW returned an unterminated string".to_string())?;
        Ok(String::from_utf16_lossy(&units[..length]))
    }

    pub(super) fn list() -> Result<Vec<ServiceRow>, String> {
        let scm = manager(SC_MANAGER_ENUMERATE_SERVICE)?;
        // Documented maximum enumeration buffer; usize supplies struct alignment.
        const BUFFER_BYTES: usize = 256 * 1024;
        let mut buffer = vec![0usize; BUFFER_BYTES / size_of::<usize>()];
        let mut resume = 0;
        let mut rows = Vec::new();
        loop {
            let previous_resume = resume;
            let mut needed = 0;
            let mut returned = 0;
            // SAFETY: buffer is writable and aligned, output pointers are valid.
            let success = unsafe {
                EnumServicesStatusExW(
                    scm.0,
                    SC_ENUM_PROCESS_INFO,
                    SERVICE_WIN32,
                    SERVICE_STATE_ALL,
                    buffer.as_mut_ptr().cast(),
                    BUFFER_BYTES as u32,
                    &mut needed,
                    &mut returned,
                    &mut resume,
                    null(),
                )
            };
            let code = if success == 0 {
                unsafe { GetLastError() }
            } else {
                0
            };
            if success == 0 && code != ERROR_MORE_DATA {
                return Err(error("EnumServicesStatusExW", code));
            }
            if returned as usize > BUFFER_BYTES / size_of::<ENUM_SERVICE_STATUS_PROCESSW>() {
                return Err("EnumServicesStatusExW returned an invalid row count".into());
            }
            // SAFETY: the API filled returned entries, within the aligned buffer.
            let entries = unsafe {
                std::slice::from_raw_parts(
                    buffer.as_ptr().cast::<ENUM_SERVICE_STATUS_PROCESSW>(),
                    returned as usize,
                )
            };
            for entry in entries {
                let status = &entry.ServiceStatusProcess;
                rows.push(ServiceRow {
                    name: buffer_string(entry.lpServiceName, &buffer)?,
                    display_name: buffer_string(entry.lpDisplayName, &buffer)?,
                    state: state_name(status.dwCurrentState),
                    // SCM only defines the PID in these states.
                    pid: if matches!(
                        status.dwCurrentState,
                        SERVICE_RUNNING
                            | SERVICE_PAUSED
                            | SERVICE_PAUSE_PENDING
                            | SERVICE_CONTINUE_PENDING
                    ) {
                        status.dwProcessId
                    } else {
                        0
                    },
                });
            }
            if success != 0 {
                break;
            }
            if returned == 0 || resume == previous_resume {
                return Err("EnumServicesStatusExW could not make progress".into());
            }
        }
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        rows.dedup_by(|a, b| a.name == b.name);
        Ok(rows)
    }

    fn query(service: &Handle) -> Result<SERVICE_STATUS_PROCESS, String> {
        let mut status = SERVICE_STATUS_PROCESS::default();
        let mut needed = 0;
        // SAFETY: status is a correctly sized, writable output structure.
        if unsafe {
            QueryServiceStatusEx(
                service.0,
                SC_STATUS_PROCESS_INFO,
                (&mut status as *mut SERVICE_STATUS_PROCESS).cast(),
                size_of::<SERVICE_STATUS_PROCESS>() as u32,
                &mut needed,
            )
        } == 0
        {
            Err(error("QueryServiceStatusEx", unsafe { GetLastError() }))
        } else {
            Ok(status)
        }
    }

    fn pending(state: u32) -> bool {
        matches!(
            state,
            SERVICE_START_PENDING
                | SERVICE_STOP_PENDING
                | SERVICE_CONTINUE_PENDING
                | SERVICE_PAUSE_PENDING
        )
    }

    fn pause(deadline: Instant, state: u32) -> Result<(), String> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!(
                "Service transition timed out after 10 seconds; last state: {}",
                state_name(state)
            ));
        }
        std::thread::sleep(remaining.min(Duration::from_millis(100)));
        Ok(())
    }

    fn stable(service: &Handle, deadline: Instant) -> Result<SERVICE_STATUS_PROCESS, String> {
        loop {
            let status = query(service)?;
            if !pending(status.dwCurrentState) {
                return Ok(status);
            }
            pause(deadline, status.dwCurrentState)?;
        }
    }

    fn wait_for(service: &Handle, target: u32, deadline: Instant) -> Result<(), String> {
        loop {
            let status = query(service)?;
            if status.dwCurrentState == target {
                return Ok(());
            }
            if !pending(status.dwCurrentState) {
                return Err(format!(
                    "Service did not reach {}; actual state: {}; exit code: {}, service exit code: {}",
                    state_name(target),
                    state_name(status.dwCurrentState),
                    status.dwWin32ExitCode,
                    status.dwServiceSpecificExitCode
                ));
            }
            pause(deadline, status.dwCurrentState)?;
        }
    }

    fn transition(service: &Handle, start: bool, deadline: Instant) -> Result<(), String> {
        let target = if start {
            SERVICE_RUNNING
        } else {
            SERVICE_STOPPED
        };
        let status = stable(service, deadline)?;
        if status.dwCurrentState == target {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("Service transition timed out after 10 seconds".into());
        }
        let (success, operation, benign_race) = if start {
            if status.dwCurrentState != SERVICE_STOPPED {
                return Err(format!(
                    "Cannot start service in state {}",
                    state_name(status.dwCurrentState)
                ));
            }
            // SAFETY: handle has SERVICE_START; no service arguments are supplied.
            (
                unsafe { StartServiceW(service.0, 0, null()) },
                "StartServiceW",
                ERROR_SERVICE_ALREADY_RUNNING,
            )
        } else {
            let mut output = SERVICE_STATUS::default();
            // SAFETY: handle has SERVICE_STOP and output is writable.
            (
                unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut output) },
                "ControlService(STOP)",
                ERROR_SERVICE_NOT_ACTIVE,
            )
        };
        if success == 0 {
            let code = unsafe { GetLastError() };
            if code != benign_race {
                return Err(error(operation, code));
            }
            // A race error is only accepted if actual status reaches the target.
        }
        wait_for(service, target, deadline)
    }

    pub(super) fn control(name: &str, action: ServiceAction) -> Result<String, String> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let scm = manager(SC_MANAGER_CONNECT)?;
        let rights = SERVICE_QUERY_STATUS
            | match action {
                ServiceAction::Start => SERVICE_START,
                ServiceAction::Stop => SERVICE_STOP,
                ServiceAction::Restart => SERVICE_START | SERVICE_STOP,
            };
        // SAFETY: name is NUL-terminated and validated by the public interface.
        let raw = unsafe { OpenServiceW(scm.0, wide.as_ptr(), rights) };
        if raw.is_null() {
            return Err(error("OpenServiceW", unsafe { GetLastError() }));
        }
        let service = Handle(raw);
        let deadline = Instant::now() + Duration::from_secs(10);
        match action {
            ServiceAction::Start => transition(&service, true, deadline)?,
            ServiceAction::Stop => transition(&service, false, deadline)?,
            ServiceAction::Restart => {
                transition(&service, false, deadline)?;
                transition(&service, true, deadline)?;
            }
        }
        let verb = match action {
            ServiceAction::Start => "started",
            ServiceAction::Stop => "stopped",
            ServiceAction::Restart => "restarted",
        };
        Ok(format!("Service {name} {verb}"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn unsupported_platform_returns_explicit_errors() {
        assert_eq!(
            list().unwrap_err(),
            "Windows services are only supported on Windows"
        );
        for action in [
            ServiceAction::Start,
            ServiceAction::Stop,
            ServiceAction::Restart,
        ] {
            assert_eq!(
                control("Spooler", action).unwrap_err(),
                "Windows services are only supported on Windows"
            );
        }
    }

    #[test]
    fn embedded_nul_is_rejected_before_opening_scm() {
        for name in ["\0", "Spooler\0other", "Spooler\0"] {
            for action in [
                ServiceAction::Start,
                ServiceAction::Stop,
                ServiceAction::Restart,
            ] {
                assert_eq!(
                    control(name, action),
                    Err("Service name must not contain NUL".into())
                );
            }
        }
    }

    #[test]
    fn empty_names_are_rejected_before_opening_scm() {
        for name in ["", " ", "\t\r\n"] {
            for action in [
                ServiceAction::Start,
                ServiceAction::Stop,
                ServiceAction::Restart,
            ] {
                assert_eq!(
                    control(name, action),
                    Err("Service name must not be empty".into())
                );
            }
        }
    }
}
