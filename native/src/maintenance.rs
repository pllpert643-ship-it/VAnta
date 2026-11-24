//! Explicit manual system maintenance. The UI owns confirmation.

#[derive(Clone, Copy, Debug)]
pub enum Operation {
    TrimFileCache,
    PurgeStandby,
    PurgePriorityZero,
}

/// Checks the current process token without activating privileges. Existing
/// thread impersonation is refused, as it is for execute().
#[cfg(windows)]
pub fn is_elevated() -> Result<bool, String> {
    windows::reject_impersonation()?;
    windows::process_is_elevated()
}

/// Runs one explicitly confirmed manual action. Confirmation belongs to the UI.
/// Privileges are enabled only on a duplicate token on a private, short-lived
/// worker thread. The process token is never modified. No automatic elevation.
#[cfg(windows)]
pub fn execute(operation: Operation) -> Result<String, String> {
    crate::release::require_system_mutations()?;
    windows::reject_impersonation()?;
    let worker = std::thread::Builder::new()
        .name("vanta-maintenance".to_owned())
        .spawn(move || execute_with(operation, &mut windows::Native))
        .map_err(|error| format!("Cannot start isolated maintenance worker: {error}"))?;
    worker.join().unwrap_or_else(|_| {
        Err("Maintenance worker panicked; action outcome is unknown. Its temporary thread token has been discarded.".to_owned())
    })
}

#[cfg(not(windows))]
pub fn is_elevated() -> Result<bool, String> {
    Err("System maintenance is unsupported on this platform; Windows is required".to_owned())
}

#[cfg(not(windows))]
pub fn execute(_operation: Operation) -> Result<String, String> {
    Err("System maintenance is unsupported on this platform; Windows is required".to_owned())
}
#[cfg(any(windows, test))]
trait PrivilegeScope {
    fn restore(&mut self) -> Result<(), String>;
}

#[cfg(any(windows, test))]
trait Adapter {
    type Scope: PrivilegeScope;
    fn is_elevated(&mut self) -> Result<bool, String>;
    fn privilege(&mut self, name: &str) -> Result<Self::Scope, String>;
    fn trim_file_cache(&mut self) -> Result<(), String>;
    fn purge_standby(&mut self, command: u32) -> Result<(), String>;
}

#[cfg(any(windows, test))]
fn execute_with<A: Adapter>(operation: Operation, adapter: &mut A) -> Result<String, String> {
    if !adapter.is_elevated()? {
        return Err("System maintenance requires an already elevated token; run as administrator manually. No automatic elevation is performed.".to_owned());
    }
    let privilege = match operation {
        Operation::TrimFileCache => "SeIncreaseQuotaPrivilege",
        Operation::PurgeStandby | Operation::PurgePriorityZero => "SeProfileSingleProcessPrivilege",
    };
    let mut scope = adapter.privilege(privilege)?;
    let result = match operation {
        Operation::TrimFileCache => adapter.trim_file_cache().map(|()| {
            "System file cache working set trimmed; pages may fault back in. No amount of freed memory is guaranteed.".to_owned()
        }),
        Operation::PurgeStandby => adapter.purge_standby(4).map(|()| {
            "Standby purge completed using an advanced undocumented native operation. No amount of freed memory is guaranteed.".to_owned()
        }),
        Operation::PurgePriorityZero => adapter.purge_standby(5).map(|()| {
            "Priority-zero standby purge completed using an advanced undocumented native operation. No amount of freed memory is guaranteed.".to_owned()
        }),
    };
    preserve_result(result, scope.restore())
}

#[cfg(any(windows, test))]
fn preserve_result<T>(result: Result<T, String>, cleanup: Result<(), String>) -> Result<T, String> {
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(format!("Operation completed, but cleanup failed: {error}")),
        (Err(error), Err(cleanup)) => Err(format!("{error}; cleanup also failed: {cleanup}")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Default)]
    struct Fake {
        elevated: bool,
        privilege_error: Option<String>,
        action_error: Option<String>,
        cleanup_error: Option<String>,
        events: Rc<RefCell<Vec<String>>>,
    }

    struct FakePrivilege(Rc<RefCell<Vec<String>>>, Option<String>);

    impl PrivilegeScope for FakePrivilege {
        fn restore(&mut self) -> Result<(), String> {
            self.0.borrow_mut().push("restore".into());
            self.1.clone().map_or(Ok(()), Err)
        }
    }

    impl Adapter for Fake {
        type Scope = FakePrivilege;
        fn is_elevated(&mut self) -> Result<bool, String> {
            self.events.borrow_mut().push("elevation".into());
            Ok(self.elevated)
        }
        fn privilege(&mut self, name: &str) -> Result<Self::Scope, String> {
            self.events.borrow_mut().push(format!("privilege:{name}"));
            if let Some(error) = &self.privilege_error {
                return Err(error.clone());
            }
            Ok(FakePrivilege(
                Rc::clone(&self.events),
                self.cleanup_error.clone(),
            ))
        }
        fn trim_file_cache(&mut self) -> Result<(), String> {
            self.events.borrow_mut().push("trim".into());
            self.action_error.clone().map_or(Ok(()), Err)
        }
        fn purge_standby(&mut self, command: u32) -> Result<(), String> {
            self.events.borrow_mut().push(format!("purge:{command}"));
            self.action_error.clone().map_or(Ok(()), Err)
        }
    }

    #[test]
    fn missing_privilege_prevents_the_action() {
        let mut fake = Fake {
            elevated: true,
            privilege_error: Some(
                "SeIncreaseQuotaPrivilege is missing (ERROR_NOT_ALL_ASSIGNED)".into(),
            ),
            ..Default::default()
        };
        let error = execute_with(Operation::TrimFileCache, &mut fake).unwrap_err();
        assert!(error.contains("SeIncreaseQuotaPrivilege"));
        assert!(error.contains("ERROR_NOT_ALL_ASSIGNED"));
        assert_eq!(
            *fake.events.borrow(),
            ["elevation", "privilege:SeIncreaseQuotaPrivilege"]
        );
    }

    #[test]
    fn action_error_survives_cleanup_and_cleanup_failure_is_also_reported() {
        for cleanup in [None, Some("restore failed".to_owned())] {
            let mut fake = Fake {
                elevated: true,
                action_error: Some("native action failed: NTSTATUS 0xC0000061".into()),
                cleanup_error: cleanup.clone(),
                ..Default::default()
            };
            let error = execute_with(Operation::PurgeStandby, &mut fake).unwrap_err();
            assert!(error.contains("NTSTATUS 0xC0000061"));
            assert_eq!(fake.events.borrow().last().unwrap(), "restore");
            assert_eq!(
                error.contains("cleanup also failed: restore failed"),
                cleanup.is_some()
            );
        }
    }

    #[test]
    fn successful_action_with_cleanup_failure_is_not_reported_as_success() {
        let mut fake = Fake {
            elevated: true,
            cleanup_error: Some("restore failed".into()),
            ..Default::default()
        };
        let error = execute_with(Operation::PurgePriorityZero, &mut fake).unwrap_err();
        assert!(error.contains("Operation completed, but cleanup failed: restore failed"));
    }

    #[test]
    fn unsupported_standby_commands_are_rejected_without_native_calls() {
        for command in [0, 1, 2, 3, 6, 7, u32::MAX] {
            assert!(
                checked_standby_command(command)
                    .unwrap_err()
                    .contains("only commands 4 and 5")
            );
        }
        assert_eq!(checked_standby_command(4), Ok(4));
        assert_eq!(checked_standby_command(5), Ok(5));
    }

    #[test]
    fn ntstatus_errors_keep_the_unsigned_code_and_explain_common_failures() {
        for status in [0, 1, 0x4000_0000] {
            assert_eq!(standby_status(status), Ok(()));
        }
        for (status, hint) in [
            (0xc000_0022u32, "access denied"),
            (0xc000_0061, "SeProfileSingleProcessPrivilege"),
            (0xc000_0002, "unsupported"),
            (0xc000_0003, "unsupported"),
            (0xc000_00bb, "unsupported"),
            (0xc000_0001, "native operation failed"),
        ] {
            let error = standby_status(status as i32).unwrap_err();
            assert!(error.contains(&format!("0x{status:08X}")));
            assert!(error.contains("Advanced undocumented"));
            assert!(error.contains(hint));
        }
    }
    #[test]
    fn each_operation_uses_only_its_required_privilege_and_command_then_restores() {
        for (operation, privilege, action) in [
            (Operation::TrimFileCache, "SeIncreaseQuotaPrivilege", "trim"),
            (
                Operation::PurgeStandby,
                "SeProfileSingleProcessPrivilege",
                "purge:4",
            ),
            (
                Operation::PurgePriorityZero,
                "SeProfileSingleProcessPrivilege",
                "purge:5",
            ),
        ] {
            let mut fake = Fake {
                elevated: true,
                ..Default::default()
            };
            let message = execute_with(operation, &mut fake).unwrap();
            assert_eq!(
                *fake.events.borrow(),
                [
                    "elevation".to_owned(),
                    format!("privilege:{privilege}"),
                    action.to_owned(),
                    "restore".to_owned()
                ]
            );
            assert!(!message.contains("bytes freed"));
            if !matches!(operation, Operation::TrimFileCache) {
                assert!(message.contains("undocumented"));
            }
        }
    }
    #[test]
    fn unelevated_requests_never_activate_privileges_or_run_actions() {
        let mut fake = Fake::default();
        let error = execute_with(Operation::TrimFileCache, &mut fake).unwrap_err();
        assert!(error.contains("already elevated"));
        assert_eq!(*fake.events.borrow(), ["elevation"]);
    }
}

#[cfg(windows)]
mod windows {
    use super::{Adapter, PrivilegeScope, preserve_result};
    use std::{ffi::c_void, marker::PhantomData, mem::size_of, ptr, rc::Rc};
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_NO_TOKEN, ERROR_NOT_ALL_ASSIGNED, ERROR_SUCCESS, FARPROC, GetLastError,
        HANDLE, HMODULE, LUID, SetLastError,
    };
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, DuplicateTokenEx, GetTokenInformation, LUID_AND_ATTRIBUTES,
        LookupPrivilegeValueW, RevertToSelf, SE_PRIVILEGE_ENABLED, SecurityImpersonation,
        TOKEN_ACCESS_MASK, TOKEN_ADJUST_PRIVILEGES, TOKEN_DUPLICATE, TOKEN_ELEVATION,
        TOKEN_IMPERSONATE, TOKEN_PRIVILEGES, TOKEN_QUERY, TokenElevation, TokenImpersonation,
    };
    use windows_sys::Win32::System::Memory::SetSystemFileCacheSize;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken, SetThreadToken,
    };

    // Declare only the loader functions needed here so the existing manifest
    // need not enable Win32_System_LibraryLoader. These are the Win32 ABI types.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> HMODULE;
        fn GetProcAddress(module: HMODULE, name: *const u8) -> FARPROC;
    }

    struct Token(HANDLE);

    impl Drop for Token {
        fn drop(&mut self) {
            // SAFETY: Every Token exclusively owns a successful token-open result.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    fn error_code(context: &str, code: u32) -> String {
        format!(
            "{context}: {} (Win32 {code})",
            std::io::Error::from_raw_os_error(code as i32)
        )
    }

    fn os_error(context: &str) -> String {
        // SAFETY: Reading this thread's last-error value has no preconditions.
        error_code(context, unsafe { GetLastError() })
    }

    pub(super) fn reject_impersonation() -> Result<(), String> {
        let mut raw = ptr::null_mut();
        // SAFETY: Current-thread pseudo-handle and valid output; no privileges adjusted.
        if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut raw) } != 0 {
            let _existing = Token(raw);
            return Err("System maintenance refuses existing thread impersonation; use a thread with the current process token".to_owned());
        }
        // SAFETY: Read immediately after OpenThreadToken.
        let code = unsafe { GetLastError() };
        if code == ERROR_NO_TOKEN {
            Ok(())
        } else {
            Err(error_code(
                "OpenThreadToken: cannot verify absence of impersonation",
                code,
            ))
        }
    }

    fn process_token(access: TOKEN_ACCESS_MASK) -> Result<Token, String> {
        let mut raw = ptr::null_mut();
        // SAFETY: Valid current-process pseudo-handle and writable HANDLE output.
        if unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut raw) } == 0 {
            return Err(os_error("OpenProcessToken"));
        }
        Ok(Token(raw))
    }

    fn token_is_elevated(token: &Token) -> Result<bool, String> {
        let mut elevation = TOKEN_ELEVATION::default();
        let mut length = 0;
        // SAFETY: TOKEN_QUERY handle, correct output structure and buffer size.
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast(),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut length,
            )
        } == 0
        {
            return Err(os_error("GetTokenInformation(TokenElevation)"));
        }
        if length != size_of::<TOKEN_ELEVATION>() as u32 {
            return Err(
                "GetTokenInformation(TokenElevation) returned an incomplete elevation record"
                    .to_owned(),
            );
        }
        Ok(elevation.TokenIsElevated != 0)
    }

    pub(super) fn process_is_elevated() -> Result<bool, String> {
        token_is_elevated(&process_token(TOKEN_QUERY)?)
    }

    pub(super) struct Native;

    impl Adapter for Native {
        type Scope = EnabledPrivilege;

        fn is_elevated(&mut self) -> Result<bool, String> {
            reject_impersonation()?;
            process_is_elevated()
        }

        fn privilege(&mut self, name: &str) -> Result<Self::Scope, String> {
            EnabledPrivilege::enable(name)
        }

        fn trim_file_cache(&mut self) -> Result<(), String> {
            // SAFETY: SIZE_T(-1), SIZE_T(-1), flags 0 is the documented cache
            // working-set flush. Called only inside the required privilege scope.
            if unsafe { SetSystemFileCacheSize(usize::MAX, usize::MAX, 0) } == 0 {
                return Err(os_error(
                    "SetSystemFileCacheSize: file-cache trim failed; SeIncreaseQuotaPrivilege is required",
                ));
            }
            Ok(())
        }

        fn purge_standby(&mut self, command: u32) -> Result<(), String> {
            let mut command = super::checked_standby_command(command)?;
            type NtSetSystemInformation = unsafe extern "system" fn(u32, *mut c_void, u32) -> i32;
            // Windows loads ntdll before application code, for the process lifetime.
            // Borrow the loaded module only: never load from a directory or free it.
            let name: Vec<u16> = "ntdll.dll\0".encode_utf16().collect();
            // SAFETY: NUL-terminated module name.
            let module = unsafe { GetModuleHandleW(name.as_ptr()) };
            if module.is_null() {
                return Err(os_error(
                    "Advanced undocumented standby purge unavailable: loaded system ntdll.dll not found",
                ));
            }
            // SAFETY: Borrowed process-lifetime module and NUL-terminated export name.
            let address = unsafe { GetProcAddress(module, c"NtSetSystemInformation".as_ptr().cast()) }
                .ok_or_else(|| os_error("Advanced undocumented standby purge unavailable: NtSetSystemInformation export missing"))?;
            // SAFETY: phnt declares this export with the exact system ABI signature.
            let function: NtSetSystemInformation = unsafe { std::mem::transmute(address) };
            // SAFETY: SystemMemoryListInformation (80) accepts a u32 command;
            // validation permits only standby commands 4 and 5, matching the Python port.
            let status = unsafe {
                function(
                    80,
                    (&mut command as *mut u32).cast(),
                    size_of::<u32>() as u32,
                )
            };
            super::standby_status(status)
        }
    }

    pub(super) struct EnabledPrivilege {
        token: Token,
        previous: Option<TOKEN_PRIVILEGES>,
        impersonating: bool,
        // A thread impersonation scope must never move to another thread.
        _thread_bound: PhantomData<Rc<()>>,
    }

    impl EnabledPrivilege {
        fn enable(name: &str) -> Result<Self, String> {
            reject_impersonation()?;
            let source = process_token(TOKEN_QUERY | TOKEN_DUPLICATE)?;
            // Recheck elevation on the same source token used for duplication.
            if !token_is_elevated(&source)? {
                return Err("System maintenance requires an already elevated token; no automatic elevation is performed".to_owned());
            }
            let mut raw = ptr::null_mut();
            // SAFETY: Valid source token and output; default security attributes.
            if unsafe {
                DuplicateTokenEx(
                    source.0,
                    TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES | TOKEN_IMPERSONATE,
                    ptr::null(),
                    SecurityImpersonation,
                    TokenImpersonation,
                    &mut raw,
                )
            } == 0
            {
                return Err(os_error(
                    "DuplicateTokenEx: cannot create isolated maintenance token",
                ));
            }
            let mut guard = Self {
                token: Token(raw),
                previous: None,
                impersonating: false,
                _thread_bound: PhantomData,
            };
            let setup = (|| {
                let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
                let mut luid = LUID::default();
                // SAFETY: NUL-terminated local privilege name and valid output.
                if unsafe { LookupPrivilegeValueW(ptr::null(), wide.as_ptr(), &mut luid) } == 0 {
                    return Err(os_error(&format!(
                        "LookupPrivilegeValueW: required privilege {name} is unavailable"
                    )));
                }
                let state = TOKEN_PRIVILEGES {
                    PrivilegeCount: 1,
                    Privileges: [LUID_AND_ATTRIBUTES {
                        Luid: luid,
                        Attributes: SE_PRIVILEGE_ENABLED,
                    }],
                };
                let mut previous = TOKEN_PRIVILEGES::default();
                let mut length = 0;
                // SAFETY: Last-error reset is required to detect successful partial assignment.
                unsafe {
                    SetLastError(ERROR_SUCCESS);
                }
                // SAFETY: One requested privilege requires at most one previous-state entry.
                let ok = unsafe {
                    AdjustTokenPrivileges(
                        guard.token.0,
                        0,
                        &state,
                        size_of::<TOKEN_PRIVILEGES>() as u32,
                        &mut previous,
                        &mut length,
                    )
                };
                // SAFETY: Read before any further OS call or formatting.
                let code = unsafe { GetLastError() };
                if ok == 0 {
                    return Err(error_code(
                        &format!("AdjustTokenPrivileges: cannot enable {name}"),
                        code,
                    ));
                }
                guard.previous = Some(previous);
                if code == ERROR_NOT_ALL_ASSIGNED {
                    return Err(format!(
                        "Required privilege {name} is missing from the elevated token (ERROR_NOT_ALL_ASSIGNED / Win32 {code}); it cannot be granted by this application"
                    ));
                }
                if code != ERROR_SUCCESS {
                    return Err(error_code(
                        &format!("AdjustTokenPrivileges: enabling {name}"),
                        code,
                    ));
                }
                // SAFETY: Null thread pointer selects the calling private worker thread.
                if unsafe { SetThreadToken(ptr::null(), guard.token.0) } == 0 {
                    return Err(os_error(
                        "SetThreadToken: cannot activate isolated maintenance token",
                    ));
                }
                guard.impersonating = true;
                Ok(())
            })();
            if let Err(error) = setup {
                return preserve_result(Err(error), guard.restore());
            }
            Ok(guard)
        }
    }

    impl PrivilegeScope for EnabledPrivilege {
        fn restore(&mut self) -> Result<(), String> {
            let mut errors = Vec::new();
            if self.impersonating {
                // SAFETY: This non-Send guard is still on its original private thread.
                if unsafe { RevertToSelf() } != 0 {
                    self.impersonating = false;
                } else {
                    errors.push(os_error("RevertToSelf failed"));
                    // SAFETY: Explicitly detach this thread's token as a fallback.
                    if unsafe { SetThreadToken(ptr::null(), ptr::null_mut()) } != 0 {
                        self.impersonating = false;
                    } else {
                        errors.push(os_error("SetThreadToken(NULL) fallback failed; isolated worker thread will exit"));
                    }
                }
            }
            if let Some(previous) = self.previous {
                if previous.PrivilegeCount == 0 {
                    self.previous = None;
                } else {
                    // SAFETY: Restore exactly the previous state on the duplicate only.
                    unsafe {
                        SetLastError(ERROR_SUCCESS);
                    }
                    let ok = unsafe {
                        AdjustTokenPrivileges(
                            self.token.0,
                            0,
                            &previous,
                            0,
                            ptr::null_mut(),
                            ptr::null_mut(),
                        )
                    };
                    // SAFETY: Read immediately, including after a nonzero return value.
                    let code = unsafe { GetLastError() };
                    if ok != 0 && code == ERROR_SUCCESS {
                        self.previous = None;
                    } else {
                        errors.push(error_code(
                            "AdjustTokenPrivileges: restoring previous privilege state failed",
                            code,
                        ));
                    }
                }
            }
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors.join("; "))
            }
        }
    }

    impl Drop for EnabledPrivilege {
        fn drop(&mut self) {
            // execute_with and setup failure paths report cleanup errors explicitly.
            // Drop also retries failed cleanup and handles panic unwinding.
            let _ = self.restore();
        }
    }
}

// phnt ntexapi.h, pinned by the Python port at commit
// b636325a6a8f7b84cdb35b6b834ed09335ffd2ff: class 80, commands 4 and 5.
#[cfg(any(windows, test))]
fn checked_standby_command(command: u32) -> Result<u32, String> {
    match command {
        4 | 5 => Ok(command),
        _ => {
            Err("Advanced undocumented standby operation accepts only commands 4 and 5".to_owned())
        }
    }
}

#[cfg(any(windows, test))]
fn standby_status(status: i32) -> Result<(), String> {
    if status >= 0 {
        return Ok(());
    }
    let hint = match status as u32 {
        0xc000_0022 | 0xc000_0061 => "access denied or SeProfileSingleProcessPrivilege not held",
        0xc000_0002 | 0xc000_0003 | 0xc000_00bb => "operation unsupported by this Windows version",
        _ => "native operation failed",
    };
    Err(format!(
        "Advanced undocumented standby purge failed: NTSTATUS 0x{:08X} ({hint})",
        status as u32
    ))
}
