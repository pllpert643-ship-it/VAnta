//! Native Windows process operations; identities use sysinfo Unix seconds.
//!
//! Mutation calls validate creation time on the very handle used for the action.
//! Mutations require a full creation FILETIME pinned at selection. This rejects
//! same-second PID reuse after pinning. Seconds-only telemetry cannot distinguish
//! reuse that already occurred before selection; memory queries may remain unpinned.

#[derive(Clone, Copy, Debug)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_time: u64,
    /// Full GetProcessTimes creation FILETIME, captured by pin_identity at selection.
    pub creation_filetime: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
pub enum PriorityClass {
    Idle,
    BelowNormal,
    Normal,
    AboveNormal,
}

/// A VirtualQueryEx region. Addresses and sizes are bytes.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MemoryRegion {
    pub base: u64,
    pub size: u64,
    pub state: String,
    pub kind: String,
    pub protection: String,
}

#[derive(Clone, Debug)]
pub struct PerformanceMemory {
    pub commit_bytes: u64,
    pub commit_limit_bytes: u64,
    pub system_cache_bytes: u64,
    pub kernel_paged_bytes: u64,
    pub kernel_nonpaged_bytes: u64,
    pub handles: u32,
    pub threads: u32,
}
#[cfg(windows)]
pub use windows::{
    affinity, memory_regions, performance_memory, pin_identity, set_affinity, set_priority,
    terminate_process, trim_working_set,
};

#[cfg(windows)]
mod windows {
    use super::{
        MemoryRegion, PerformanceMemory, PriorityClass, ProcessIdentity, filetime_to_unix_seconds,
    };
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, FILETIME, HANDLE};
    use windows_sys::Win32::System::Memory::*;
    use windows_sys::Win32::System::ProcessStatus::{
        EmptyWorkingSet, GetPerformanceInfo, PERFORMANCE_INFORMATION,
    };
    use windows_sys::Win32::System::Threading::{
        ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, GetProcessAffinityMask,
        GetProcessTimes, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, OpenProcess,
        PROCESS_ACCESS_RIGHTS, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SET_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE, SetPriorityClass,
        SetProcessAffinityMask, TerminateProcess,
    };

    struct ProcessHandle(HANDLE, u64);

    impl Drop for ProcessHandle {
        fn drop(&mut self) {
            // SAFETY: OpenProcess transferred ownership of this non-null handle.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    fn os_error(context: &str) -> String {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(5) {
            format!("{context}: access denied ({error}); no elevation was attempted")
        } else {
            format!("{context}: {error}")
        }
    }

    fn open_verified(
        target: ProcessIdentity,
        access: PROCESS_ACCESS_RIGHTS,
        mutation: bool,
    ) -> Result<ProcessHandle, String> {
        if mutation && (target.pid == 0 || target.pid == 4 || target.pid == std::process::id()) {
            return Err(format!(
                "Cannot mutate protected process PID {}",
                target.pid
            ));
        }
        if mutation && target.creation_filetime.is_none() {
            return Err("Process identity is not pinned; call pin_identity at live selection before requesting a mutation".to_owned());
        }
        // SAFETY: Valid scalar arguments; inheritance is explicitly disabled.
        let raw = unsafe { OpenProcess(access | PROCESS_QUERY_LIMITED_INFORMATION, 0, target.pid) };
        if raw.is_null() {
            return Err(os_error(&format!(
                "OpenProcess PID {} with access {:#x}",
                target.pid,
                access | PROCESS_QUERY_LIMITED_INFORMATION
            )));
        }
        let mut handle = ProcessHandle(raw, 0);
        handle.1 = verify_action_handle(handle.0, target, mutation)?;
        Ok(handle)
    }

    fn verify_action_handle(
        handle: HANDLE,
        target: ProcessIdentity,
        mutation: bool,
    ) -> Result<u64, String> {
        if mutation && target.creation_filetime.is_none() {
            return Err(
                "Process identity is not pinned; call pin_identity before mutation".to_owned(),
            );
        }
        let ticks = creation_time(handle)?;
        let actual_start = filetime_to_unix_seconds(ticks)?;
        if actual_start != target.start_time {
            return Err(format!(
                "PID {} identity mismatch (PID may have been reused): expected start_time {}, found {}; select the process again",
                target.pid, target.start_time, actual_start
            ));
        }
        if target
            .creation_filetime
            .is_some_and(|expected| expected != ticks)
        {
            return Err(format!(
                "PID {} creation FILETIME mismatch (PID may have been reused); select and pin the process again",
                target.pid
            ));
        }
        Ok(ticks)
    }

    /// Pin a live selection. Existing full tokens are verified rather than replaced.
    pub fn pin_identity(target: ProcessIdentity) -> Result<ProcessIdentity, String> {
        let handle = open_verified(target, PROCESS_QUERY_LIMITED_INFORMATION, false)?;
        Ok(ProcessIdentity {
            creation_filetime: Some(handle.1),
            ..target
        })
    }
    fn creation_time(handle: HANDLE) -> Result<u64, String> {
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // SAFETY: The caller holds the handle; all outputs are valid FILETIMEs.
        if unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
            == 0
        {
            return Err(os_error(
                "GetProcessTimes: cannot validate process identity",
            ));
        }
        let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        Ok(ticks)
    }

    pub fn performance_memory() -> Result<PerformanceMemory, String> {
        let mut info = PERFORMANCE_INFORMATION {
            cb: std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32,
            ..Default::default()
        };
        // SAFETY: info is a writable structure with the required size field.
        if unsafe {
            GetPerformanceInfo(
                &mut info,
                std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(os_error(
                "GetPerformanceInfo: read-only system memory query",
            ));
        }
        if info.PageSize == 0 {
            return Err("GetPerformanceInfo returned a zero page size".to_owned());
        }
        let bytes = |pages: usize| -> Result<u64, String> {
            (pages as u64)
                .checked_mul(info.PageSize as u64)
                .ok_or_else(|| {
                    "GetPerformanceInfo memory counter overflow converting pages to bytes"
                        .to_owned()
                })
        };
        Ok(PerformanceMemory {
            commit_bytes: bytes(info.CommitTotal)?,
            commit_limit_bytes: bytes(info.CommitLimit)?,
            system_cache_bytes: bytes(info.SystemCache)?,
            kernel_paged_bytes: bytes(info.KernelPaged)?,
            kernel_nonpaged_bytes: bytes(info.KernelNonpaged)?,
            handles: info.HandleCount,
            threads: info.ThreadCount,
        })
    }
    /// Enumerate regions without reading their contents. Returns an error rather
    /// than a partial inventory if the safety cap or an unexpected query fails.
    /// A 32-bit build can enumerate only its representable address range.
    pub fn memory_regions(target: ProcessIdentity) -> Result<Vec<MemoryRegion>, String> {
        const MAX_ITERATIONS: usize = 65_536;
        const MAX_RESULTS: usize = 65_536;
        let handle = open_verified(target, PROCESS_QUERY_INFORMATION, false)?;
        let mut regions = Vec::new();
        let mut address = 0usize;
        for _ in 0..MAX_ITERATIONS {
            let mut info = MEMORY_BASIC_INFORMATION::default();
            // SAFETY: Querying metadata only; info is writable and correctly sized.
            let written = unsafe {
                VirtualQueryEx(
                    handle.0,
                    address as *const _,
                    &mut info,
                    std::mem::size_of_val(&info),
                )
            };
            if written == 0 {
                let error = std::io::Error::last_os_error();
                // VirtualQueryEx signals an address above the target's maximum
                // with ERROR_INVALID_PARAMETER, including WOW64 targets.
                if address > 0 && error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                    return Ok(regions);
                }
                return Err(format!(
                    "VirtualQueryEx PID {} at {address:#x}: read-only memory query failed; required access PROCESS_QUERY_INFORMATION: {error}",
                    target.pid
                ));
            }
            if written != std::mem::size_of_val(&info) {
                return Err(format!(
                    "VirtualQueryEx PID {} returned an incomplete region",
                    target.pid
                ));
            }
            let base = info.BaseAddress as usize;
            let next = base
                .checked_add(info.RegionSize)
                .filter(|next| info.RegionSize > 0 && base <= address && *next > address)
                .ok_or_else(|| {
                    format!(
                        "VirtualQueryEx PID {}: invalid region or address overflow at {address:#x}",
                        target.pid
                    )
                })?;
            if regions.len() >= MAX_RESULTS {
                return Err(format!(
                    "VirtualQueryEx PID {} exceeded the {MAX_RESULTS} region result limit",
                    target.pid
                ));
            }
            let state = match info.State {
                MEM_COMMIT => "Committed".to_owned(),
                MEM_RESERVE => "Reserved".to_owned(),
                MEM_FREE => "Free".to_owned(),
                value => format!("Unknown ({value:#x})"),
            };
            // Type is undefined for free regions; Protect is undefined unless committed.
            let kind = if info.State == MEM_FREE {
                "None".to_owned()
            } else {
                match info.Type {
                    MEM_IMAGE => "Image".to_owned(),
                    MEM_MAPPED => "Mapped".to_owned(),
                    MEM_PRIVATE => "Private".to_owned(),
                    0 => "None".to_owned(),
                    value => format!("Unknown ({value:#x})"),
                }
            };
            let protection = if info.State == MEM_COMMIT {
                protection_name(info.Protect)
            } else {
                "None".to_owned()
            };
            regions.push(MemoryRegion {
                base: base as u64,
                size: info.RegionSize as u64,
                state,
                kind,
                protection,
            });
            address = next;
        }
        Err(format!(
            "VirtualQueryEx PID {} exceeded the {MAX_ITERATIONS} iteration limit",
            target.pid
        ))
    }

    fn protection_name(flags: u32) -> String {
        let mut name = match flags & 0xff {
            0 => "None".to_owned(),
            PAGE_NOACCESS => "No access".to_owned(),
            PAGE_READONLY => "Read".to_owned(),
            PAGE_READWRITE => "Read/write".to_owned(),
            PAGE_WRITECOPY => "Write copy".to_owned(),
            PAGE_EXECUTE => "Execute".to_owned(),
            PAGE_EXECUTE_READ => "Execute/read".to_owned(),
            PAGE_EXECUTE_READWRITE => "Execute/read/write".to_owned(),
            PAGE_EXECUTE_WRITECOPY => "Execute/write copy".to_owned(),
            value => format!("Unknown ({value:#x})"),
        };
        for (flag, label) in [
            (PAGE_GUARD, "Guard"),
            (PAGE_NOCACHE, "No cache"),
            (PAGE_WRITECOMBINE, "Write combine"),
        ] {
            if flags & flag != 0 {
                name.push_str(" | ");
                name.push_str(label);
            }
        }
        let other = flags & !(0xff | PAGE_GUARD | PAGE_NOCACHE | PAGE_WRITECOMBINE);
        if other != 0 {
            name.push_str(&format!(" | {other:#x}"));
        }
        name
    }
    fn handle_affinity(handle: HANDLE) -> Result<(usize, usize), String> {
        let mut process_mask = 0usize;
        let mut system_mask = 0usize;
        // SAFETY: Verified query handle and writable pointer-sized mask outputs.
        if unsafe { GetProcessAffinityMask(handle, &mut process_mask, &mut system_mask) } == 0 {
            return Err(os_error("GetProcessAffinityMask: read-only affinity query"));
        }
        if process_mask == 0 || system_mask == 0 {
            return Err("GetProcessAffinityMask returned empty masks; this process may span multiple processor groups, which this affinity interface cannot represent".to_owned());
        }
        Ok((process_mask, system_mask))
    }

    /// Returns (process_mask, system_mask), within the Win32 processor-group and
    /// pointer-width limits. Does not enumerate or modify multiple groups.
    pub fn affinity(target: ProcessIdentity) -> Result<(usize, usize), String> {
        let handle = open_verified(target, PROCESS_QUERY_INFORMATION, false)?;
        handle_affinity(handle.0)
    }

    /// Applies a nonzero subset of the system mask, on one verified handle.
    pub fn set_affinity(target: ProcessIdentity, mask: usize) -> Result<String, String> {
        let handle = open_verified(
            target,
            PROCESS_QUERY_INFORMATION | PROCESS_SET_INFORMATION,
            true,
        )?;
        if mask == 0 {
            return Err("Process affinity mask must be nonzero".to_owned());
        }
        let (_, system_mask) = handle_affinity(handle.0)?;
        if mask & !system_mask != 0 {
            return Err(format!(
                "Affinity mask {mask:#x} contains processors outside the system affinity mask {system_mask:#x}"
            ));
        }
        // SAFETY: Same pinned process handle, with set-information access, and a
        // nonzero mask restricted to the system mask from this handle's query.
        if unsafe { SetProcessAffinityMask(handle.0, mask) } == 0 {
            return Err(os_error(&format!(
                "SetProcessAffinityMask PID {}; processor-group constraints may prevent this mask",
                target.pid
            )));
        }
        Ok(format!("PID {} affinity set to {mask:#x}", target.pid))
    }
    pub fn terminate_process(target: ProcessIdentity) -> Result<String, String> {
        let handle = open_verified(target, PROCESS_TERMINATE, true)?;
        // SAFETY: This handle has terminate access and passed identity validation.
        if unsafe { TerminateProcess(handle.0, 1) } == 0 {
            return Err(os_error(&format!("TerminateProcess PID {}", target.pid)));
        }
        // TerminateProcess is asynchronous: report the request, not completion.
        Ok(format!("Termination requested for PID {}", target.pid))
    }

    pub fn set_priority(target: ProcessIdentity, level: PriorityClass) -> Result<String, String> {
        let handle = open_verified(target, PROCESS_SET_INFORMATION, true)?;
        let class = match level {
            PriorityClass::Idle => IDLE_PRIORITY_CLASS,
            PriorityClass::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
            PriorityClass::Normal => NORMAL_PRIORITY_CLASS,
            PriorityClass::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
        };
        // SAFETY: This verified handle has set-information access.
        if unsafe { SetPriorityClass(handle.0, class) } == 0 {
            return Err(os_error(&format!("SetPriorityClass PID {}", target.pid)));
        }
        Ok(format!("PID {} priority set to {level:?}", target.pid))
    }

    pub fn trim_working_set(target: ProcessIdentity) -> Result<String, String> {
        let handle = open_verified(target, PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION, true)?;
        // SAFETY: This verified handle has query-information and set-quota access.
        if unsafe { EmptyWorkingSet(handle.0) } == 0 {
            return Err(os_error(&format!("EmptyWorkingSet PID {}", target.pid)));
        }
        Ok(format!("Working set trimmed for PID {}", target.pid))
    }
}

#[cfg(not(windows))]
pub fn terminate_process(_target: ProcessIdentity) -> Result<String, String> {
    Err("Process termination is unsupported on this platform; Windows is required".to_owned())
}

#[cfg(not(windows))]
pub fn set_priority(_target: ProcessIdentity, _level: PriorityClass) -> Result<String, String> {
    Err("Process priority changes are unsupported on this platform; Windows is required".to_owned())
}

#[cfg(not(windows))]
pub fn trim_working_set(_target: ProcessIdentity) -> Result<String, String> {
    Err("Working set trimming is unsupported on this platform; Windows is required".to_owned())
}

#[cfg(any(windows, test))]
fn filetime_to_unix_seconds(ticks: u64) -> Result<u64, String> {
    (ticks / 10_000_000)
        .checked_sub(11_644_473_600)
        .ok_or_else(|| "Process creation time predates the Unix epoch".to_owned())
}

#[cfg(test)]
mod tests {
    #[test]
    fn filetime_conversion_matches_sysinfo_unix_seconds() {
        assert_eq!(
            super::filetime_to_unix_seconds(116_444_736_000_000_000),
            Ok(0)
        );
        assert_eq!(
            super::filetime_to_unix_seconds(116_444_736_019_999_999),
            Ok(1)
        );
        assert!(super::filetime_to_unix_seconds(0).is_err());
    }
}

#[cfg(all(test, windows))]
mod process_tests {
    use super::*;

    struct OwnedChild(std::process::Child);

    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn child() -> (OwnedChild, ProcessIdentity) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::GetProcessTimes;
        let helper = concat!(module_path!(), "::owned_child")
            .split_once("::")
            .unwrap()
            .1;
        let child = OwnedChild(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", helper, "--ignored"])
                .env("VANTA_PLATFORM_OWNED_CHILD", "1")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // SAFETY: Child owns this process handle and all outputs are valid.
        assert_ne!(
            unsafe {
                GetProcessTimes(
                    child.0.as_raw_handle(),
                    &mut created,
                    &mut exited,
                    &mut kernel,
                    &mut user,
                )
            },
            0
        );
        let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        let target = ProcessIdentity {
            pid: child.0.id(),
            start_time: ticks / 10_000_000 - 11_644_473_600,
            creation_filetime: None,
        };
        (child, pin_identity(target).unwrap())
    }

    #[test]
    fn affinity_validates_masks_and_changes_only_the_owned_child() {
        let (mut child, target) = child();
        let (original, system) = affinity(target).unwrap();
        assert_ne!(original, 0);
        assert_eq!(original & !system, 0);
        let unpinned = ProcessIdentity {
            creation_filetime: None,
            ..target
        };
        assert!(
            set_affinity(unpinned, original)
                .unwrap_err()
                .contains("pin_identity")
        );
        assert!(set_affinity(target, 0).unwrap_err().contains("nonzero"));
        if !system != 0 {
            let outside = 1usize << (!system).trailing_zeros();
            assert!(
                set_affinity(target, outside)
                    .unwrap_err()
                    .contains("system affinity mask")
            );
        }
        let single = 1usize << original.trailing_zeros();
        set_affinity(target, single).unwrap();
        assert_eq!(affinity(target).unwrap().0, single);
        set_affinity(target, original).unwrap();
        assert_eq!(affinity(target).unwrap().0, original);
        let ticks = target.creation_filetime.unwrap();
        let altered = if ticks % 10_000_000 == 0 {
            ticks + 1
        } else {
            ticks - 1
        };
        let stale = ProcessIdentity {
            creation_filetime: Some(altered),
            ..target
        };
        assert!(
            set_affinity(stale, single)
                .unwrap_err()
                .contains("creation FILETIME mismatch")
        );
        assert!(
            affinity(stale)
                .unwrap_err()
                .contains("creation FILETIME mismatch")
        );
        assert!(child.0.try_wait().unwrap().is_none());
    }
    #[test]
    fn unpinned_mutations_are_refused_and_full_token_mismatch_is_rejected() {
        let (mut child, target) = child();
        let unpinned = ProcessIdentity {
            creation_filetime: None,
            ..target
        };
        assert!(!memory_regions(unpinned).unwrap().is_empty());
        for result in [
            terminate_process(unpinned),
            set_priority(unpinned, PriorityClass::Idle),
            trim_working_set(unpinned),
        ] {
            assert!(result.unwrap_err().contains("pin_identity"));
        }
        let ticks = target.creation_filetime.unwrap();
        // A different 100ns token deliberately in the same second: seconds-only
        // validation would accept it. Do not require the OS to actually reuse a PID.
        let altered = if ticks % 10_000_000 == 0 {
            ticks + 1
        } else {
            ticks - 1
        };
        assert_eq!(ticks / 10_000_000, altered / 10_000_000);
        let stale = ProcessIdentity {
            creation_filetime: Some(altered),
            ..target
        };
        for result in [
            terminate_process(stale),
            set_priority(stale, PriorityClass::Idle),
            trim_working_set(stale),
        ] {
            assert!(result.unwrap_err().contains("creation FILETIME mismatch"));
        }
        assert!(
            pin_identity(stale)
                .unwrap_err()
                .contains("creation FILETIME mismatch")
        );
        assert!(
            memory_regions(stale)
                .unwrap_err()
                .contains("creation FILETIME mismatch")
        );
        assert!(child.0.try_wait().unwrap().is_none());
    }
    #[test]
    fn performance_snapshot_reports_bytes_and_system_counts() {
        use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
        let snapshot = performance_memory().unwrap();
        let mut info = SYSTEM_INFO::default();
        // SAFETY: info is a valid writable output buffer.
        unsafe {
            GetSystemInfo(&mut info);
        }
        let page = u64::from(info.dwPageSize);
        assert!(page > 0);
        assert!(snapshot.commit_bytes > 0);
        assert!(snapshot.commit_limit_bytes >= snapshot.commit_bytes);
        assert!(snapshot.handles > 0 && snapshot.threads > 0);
        for value in [
            snapshot.commit_bytes,
            snapshot.commit_limit_bytes,
            snapshot.system_cache_bytes,
            snapshot.kernel_paged_bytes,
            snapshot.kernel_nonpaged_bytes,
        ] {
            assert_eq!(
                value % page,
                0,
                "memory counters must be converted to bytes"
            );
        }
    }
    #[test]
    fn memory_inventory_is_ordered_and_rejects_stale_identity() {
        let (_child, target) = child();
        let regions = memory_regions(target).unwrap();
        assert!(!regions.is_empty());
        assert!(regions.len() <= 65_536);
        assert!(regions.iter().all(|r| r.size > 0
            && !r.state.is_empty()
            && !r.kind.is_empty()
            && !r.protection.is_empty()));
        for pair in regions.windows(2) {
            assert_eq!(pair[0].base.checked_add(pair[0].size), Some(pair[1].base));
        }
        let stale = ProcessIdentity {
            start_time: target.start_time + 1,
            ..target
        };
        assert!(
            memory_regions(stale)
                .unwrap_err()
                .contains("identity mismatch")
        );
        assert!(
            memory_regions(ProcessIdentity {
                pid: 0,
                start_time: 0,
                creation_filetime: None,
            })
            .unwrap_err()
            .contains("access")
        );
    }
    #[test]
    #[ignore = "helper process launched only by the owned-child tests"]
    fn owned_child() {
        assert_eq!(
            std::env::var("VANTA_PLATFORM_OWNED_CHILD").as_deref(),
            Ok("1")
        );
        // Bounded lifetime also cleans up if the parent test runner crashes.
        std::thread::sleep(std::time::Duration::from_secs(30));
    }

    #[test]
    fn reused_pid_identity_is_rejected_without_mutating_child() {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::Threading::GetPriorityClass;
        let (mut child, target) = child();
        // SAFETY: Child owns this handle throughout the assertion.
        let original_priority = unsafe { GetPriorityClass(child.0.as_raw_handle()) };
        let stale = ProcessIdentity {
            start_time: target.start_time + 1,
            ..target
        };
        for result in [
            terminate_process(stale),
            set_priority(stale, PriorityClass::Idle),
            trim_working_set(stale),
        ] {
            assert!(result.unwrap_err().contains("identity mismatch"));
        }
        assert!(child.0.try_wait().unwrap().is_none());
        // SAFETY: Child still owns the handle.
        assert_eq!(
            unsafe { GetPriorityClass(child.0.as_raw_handle()) },
            original_priority
        );
    }

    #[test]
    fn verified_child_supports_safe_priorities_trim_and_termination() {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::Threading::{
            ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, GetPriorityClass,
            IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
        };
        let (mut child, target) = child();
        for (level, expected) in [
            (PriorityClass::Idle, IDLE_PRIORITY_CLASS),
            (PriorityClass::BelowNormal, BELOW_NORMAL_PRIORITY_CLASS),
            (PriorityClass::Normal, NORMAL_PRIORITY_CLASS),
            (PriorityClass::AboveNormal, ABOVE_NORMAL_PRIORITY_CLASS),
        ] {
            set_priority(target, level).unwrap();
            // SAFETY: Child owns the handle.
            assert_eq!(
                unsafe { GetPriorityClass(child.0.as_raw_handle()) },
                expected
            );
        }
        trim_working_set(target).unwrap();
        assert!(child.0.try_wait().unwrap().is_none());
        terminate_process(target).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while child.0.try_wait().unwrap().is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "termination did not complete"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    #[test]
    fn mutations_reject_protected_pids_before_opening() {
        for pid in [0, 4, std::process::id()] {
            let target = ProcessIdentity {
                pid,
                start_time: 0,
                creation_filetime: None,
            };
            assert!(terminate_process(target).unwrap_err().contains("protected"));
            assert!(
                set_priority(target, PriorityClass::Normal)
                    .unwrap_err()
                    .contains("protected")
            );
            assert!(trim_working_set(target).unwrap_err().contains("protected"));
            assert!(set_affinity(target, 1).unwrap_err().contains("protected"));
        }
    }
}

#[cfg(not(windows))]
pub fn memory_regions(_target: ProcessIdentity) -> Result<Vec<MemoryRegion>, String> {
    Err("Process memory queries are unsupported on this platform; Windows is required".to_owned())
}

#[cfg(not(windows))]
pub fn performance_memory() -> Result<PerformanceMemory, String> {
    Err("System memory queries are unsupported on this platform; Windows is required".to_owned())
}

#[cfg(not(windows))]
pub fn pin_identity(_target: ProcessIdentity) -> Result<ProcessIdentity, String> {
    Err("Process identity pinning is unsupported on this platform; Windows is required".to_owned())
}

#[cfg(not(windows))]
pub fn affinity(_target: ProcessIdentity) -> Result<(usize, usize), String> {
    Err("Process affinity queries are unsupported on this platform; Windows is required".to_owned())
}

#[cfg(not(windows))]
pub fn set_affinity(_target: ProcessIdentity, _mask: usize) -> Result<String, String> {
    Err("Process affinity changes are unsupported on this platform; Windows is required".to_owned())
}
