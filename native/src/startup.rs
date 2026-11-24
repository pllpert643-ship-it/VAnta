//! Run-key and Startup-folder inventory, with narrowly scoped HKCU approval edits.
//!
//! Unlike the Python backup/remove implementation, this module leaves Run commands
//! intact and edits only Explorer's StartupApproved record, as requested for the
//! native migration. That binary format is not a documented Windows API contract.
//! Inventory never creates keys; listing unsupported approval formats yields None.
//! Mutations normalize supported records to 12 bytes with a zero timestamp.
//! Command comparison is a best-effort stale-action check, not an atomic registry
//! compare-and-swap; another application can still change Run immediately afterward.

#[derive(Clone, Debug)]
pub struct StartupEntry {
    pub scope: String,
    pub name: String,
    pub command: String,
    pub enabled: Option<bool>,
    /// This module supports editing this entry; Windows can still deny access.
    pub writable: bool,
}

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";
const MAX_TEXT_UNITS: usize = 32_767;

fn validate_text(text: &str, label: &str) -> Result<(), String> {
    if text.is_empty() || text.contains('\0') || text.encode_utf16().count() > MAX_TEXT_UNITS {
        return Err(format!(
            "Invalid startup {label}: empty, NUL, or overlong text"
        ));
    }
    Ok(())
}

/// Keep unknown, malformed and truncated records distinct from enabled entries.
fn approval_state(bytes: &[u8]) -> Option<bool> {
    if bytes.len() != 12 || bytes[1..4] != [0, 0, 0] {
        return None;
    }
    match bytes[0] {
        2 | 6 => Some(true),
        3 | 7 => Some(false),
        _ => None,
    }
}

fn approval_payload(existing: Option<&[u8]>, enabled: bool) -> Result<[u8; 12], String> {
    if existing.is_some_and(|bytes| approval_state(bytes).is_none()) {
        return Err("Unsupported StartupApproved payload; use Windows Startup apps".into());
    }
    // Intentionally reset the timestamp, per the requested native format. Unknown
    // extensions are refused above instead of silently discarding their payload.
    let mut bytes = [0; 12];
    bytes[0] = if enabled { 2 } else { 3 };
    Ok(bytes)
}

fn validate_mutation(entry: &StartupEntry, current_command: &str) -> Result<(), String> {
    if !entry.writable
        || !matches!(
            entry.scope.as_str(),
            "HKCU Run (64-bit)" | "HKCU Run (32-bit)"
        )
    {
        return Err("Only current-user Run entries are writable; refresh the startup list".into());
    }
    validate_text(&entry.name, "name")?;
    validate_text(&entry.command, "command")?;
    if entry.command != current_command {
        return Err("Startup command changed; refresh before trying again".into());
    }
    Ok(())
}

#[cfg(windows)]
pub use windows::list;

pub fn set_enabled(entry: &StartupEntry, enabled: bool) -> Result<String, String> {
    crate::release::require_system_mutations()?;
    #[cfg(windows)]
    {
        windows::set_enabled(entry, enabled)
    }
    #[cfg(not(windows))]
    {
        let _ = (entry, enabled);
        Err("Startup changes require Windows".into())
    }
}

#[cfg(not(windows))]
pub fn list() -> Result<Vec<StartupEntry>, String> {
    Err("Startup inventory requires Windows".into())
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{collections::HashSet, ffi::c_void, path::PathBuf, ptr};

    type Hkey = *mut c_void;
    const HKCU: Hkey = 0x80000001_u32 as i32 as isize as Hkey;
    const HKLM: Hkey = 0x80000002_u32 as i32 as isize as Hkey;
    const QUERY: u32 = 0x0001;
    const SET: u32 = 0x0002;
    const VIEW64: u32 = 0x0100;
    const VIEW32: u32 = 0x0200;
    const NOT_FOUND: i32 = 2;
    const PATH_NOT_FOUND: i32 = 3;
    const MORE_DATA: i32 = 234;
    const NO_MORE_ITEMS: i32 = 259;
    const REG_SZ: u32 = 1;
    const REG_EXPAND_SZ: u32 = 2;
    const REG_BINARY: u32 = 3;
    const MAX_VALUE_BYTES: usize = 1024 * 1024;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegOpenKeyExW(
            key: Hkey,
            subkey: *const u16,
            options: u32,
            access: u32,
            result: *mut Hkey,
        ) -> i32;
        fn RegCloseKey(key: Hkey) -> i32;
        fn RegEnumValueW(
            key: Hkey,
            index: u32,
            name: *mut u16,
            name_len: *mut u32,
            reserved: *mut u32,
            kind: *mut u32,
            data: *mut u8,
            size: *mut u32,
        ) -> i32;
        fn RegQueryValueExW(
            key: Hkey,
            name: *const u16,
            reserved: *mut u32,
            kind: *mut u32,
            data: *mut u8,
            size: *mut u32,
        ) -> i32;
        fn RegCreateKeyExW(
            key: Hkey,
            subkey: *const u16,
            reserved: u32,
            class: *mut u16,
            options: u32,
            access: u32,
            security: *const c_void,
            result: *mut Hkey,
            disposition: *mut u32,
        ) -> i32;
        fn RegSetValueExW(
            key: Hkey,
            name: *const u16,
            reserved: u32,
            kind: u32,
            data: *const u8,
            size: u32,
        ) -> i32;
    }

    struct Key(Hkey);
    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: this wrapper owns a successfully opened/created key.
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }
    fn error(operation: &str, code: i32) -> String {
        format!(
            "{operation}: {} (registry status {code})",
            std::io::Error::from_raw_os_error(code)
        )
    }
    fn open(hive: Hkey, path: &str, access: u32) -> Result<Option<Key>, String> {
        let mut key = ptr::null_mut();
        let path = wide(path);
        // SAFETY: path is terminated and result is writable. No key is created.
        let status = unsafe { RegOpenKeyExW(hive, path.as_ptr(), 0, access, &mut key) };
        match status {
            0 => Ok(Some(Key(key))),
            NOT_FOUND | PATH_NOT_FOUND => Ok(None),
            _ => Err(error("Open startup key", status)),
        }
    }
    fn query(key: &Key, name: &str) -> Result<Option<(u32, Vec<u8>)>, String> {
        let name = wide(name);
        // Bounded retries handle a value changing size between the two calls.
        for _ in 0..4 {
            let (mut kind, mut size) = (0, 0);
            // SAFETY: a live key, terminated name and writable scalar outputs.
            let status = unsafe {
                RegQueryValueExW(
                    key.0,
                    name.as_ptr(),
                    ptr::null_mut(),
                    &mut kind,
                    ptr::null_mut(),
                    &mut size,
                )
            };
            if status == NOT_FOUND {
                return Ok(None);
            }
            if status != 0 && status != MORE_DATA {
                return Err(error("Query startup value size", status));
            }
            if size as usize > MAX_VALUE_BYTES {
                return Err("Startup value exceeds 1 MiB limit".into());
            }
            let mut bytes = vec![0; size as usize];
            // SAFETY: bytes has the requested capacity and size describes it.
            let status = unsafe {
                RegQueryValueExW(
                    key.0,
                    name.as_ptr(),
                    ptr::null_mut(),
                    &mut kind,
                    bytes.as_mut_ptr(),
                    &mut size,
                )
            };
            if status == MORE_DATA {
                continue;
            }
            if status == NOT_FOUND {
                return Ok(None);
            }
            if status != 0 {
                return Err(error("Query startup value", status));
            }
            if size as usize > bytes.len() {
                return Err("Registry returned an invalid value size".into());
            }
            bytes.truncate(size as usize);
            return Ok(Some((kind, bytes)));
        }
        Err("Startup value kept changing; refresh and retry".into())
    }
    fn command(kind: u32, bytes: &[u8]) -> Result<String, String> {
        if !matches!(kind, REG_SZ | REG_EXPAND_SZ) || !bytes.len().is_multiple_of(2) {
            return Err("Run command must be a UTF-16 registry string".into());
        }
        let mut units: Vec<_> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        if units.last() == Some(&0) {
            units.pop();
        }
        let text = String::from_utf16(&units).map_err(|_| "Run command contains invalid UTF-16")?;
        validate_text(&text, "command")?;
        Ok(text)
    }
    fn read_state(hive: Hkey, section: &str, name: &str) -> Option<bool> {
        if crate::release::STORE {
            return None;
        }
        // Approval state is optional evidence. Denied/unrecognized is None.
        let key = match open(hive, &format!(r"{APPROVED}\{section}"), QUERY | VIEW64) {
            Ok(None) => return Some(true),
            Ok(Some(key)) => key,
            Err(_) => return None,
        };
        match query(&key, name) {
            Ok(None) => Some(true),
            Ok(Some((REG_BINARY, bytes))) => approval_state(&bytes),
            _ => None,
        }
    }
    pub fn list() -> Result<Vec<StartupEntry>, String> {
        let mut entries = Vec::new();
        let mut seen_user = HashSet::new();
        for (hive, label) in [(HKCU, "HKCU"), (HKLM, "HKLM")] {
            for (view, bits) in [(VIEW64, "64"), (VIEW32, "32")] {
                let Some(key) = open(hive, RUN, QUERY | view)? else {
                    continue;
                };
                let mut complete = false;
                for index in 0..100_000 {
                    // Maximum registry value name is 16,383 UTF-16 code units.
                    let mut name = vec![0_u16; 16_384];
                    let mut len = name.len() as u32;
                    // SAFETY: name is writable; optional value outputs are null.
                    let status = unsafe {
                        RegEnumValueW(
                            key.0,
                            index,
                            name.as_mut_ptr(),
                            &mut len,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            ptr::null_mut(),
                            ptr::null_mut(),
                        )
                    };
                    if status == NO_MORE_ITEMS {
                        complete = true;
                        break;
                    }
                    if status != 0 {
                        return Err(error("Enumerate Run values", status));
                    }
                    if len as usize >= name.len() {
                        return Err("Registry returned an invalid value name length".into());
                    }
                    let name = String::from_utf16(&name[..len as usize])
                        .map_err(|_| "Invalid UTF-16 startup name")?;
                    if name.is_empty() {
                        continue;
                    } // unnamed Run value is not a launch entry
                    let Some((kind, bytes)) = query(&key, &name)? else {
                        continue;
                    };
                    let Ok(command) = command(kind, &bytes) else {
                        continue;
                    };
                    // HKCU Run is shared across WOW64 views. Preserve distinct HKLM views.
                    if label == "HKCU" && !seen_user.insert((name.to_lowercase(), command.clone()))
                    {
                        continue;
                    }
                    let section = if label == "HKLM" && bits == "32" {
                        "Run32"
                    } else {
                        "Run"
                    };
                    let enabled = read_state(hive, section, &name);
                    entries.push(StartupEntry {
                        scope: format!("{label} Run ({bits}-bit)"),
                        name,
                        command,
                        enabled,
                        writable: !crate::release::STORE && label == "HKCU" && enabled.is_some(),
                    });
                }
                if !complete {
                    return Err("Startup inventory exceeds 100,000 values".into());
                }
            }
        }
        for (variable, label, hive) in [
            ("APPDATA", "Current user Startup folder", HKCU),
            ("PROGRAMDATA", "All users Startup folder", HKLM),
        ] {
            let Some(root) = std::env::var_os(variable) else {
                continue;
            };
            let folder = PathBuf::from(root).join(r"Microsoft\Windows\Start Menu\Programs\Startup");
            let files = match std::fs::read_dir(&folder) {
                Ok(files) => files,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("Read startup folder: {e}")),
            };
            for file in files {
                let file = file.map_err(|e| format!("Read startup folder entry: {e}"))?;
                if !file.file_type().map_err(|e| e.to_string())?.is_file() {
                    continue;
                }
                let name = file.file_name().to_string_lossy().into_owned();
                if name.eq_ignore_ascii_case("desktop.ini") {
                    continue;
                }
                if entries.len() >= 100_000 {
                    return Err("Too many startup entries".into());
                }
                entries.push(StartupEntry {
                    scope: label.into(),
                    command: file.path().to_string_lossy().into_owned(),
                    enabled: read_state(hive, "StartupFolder", &name),
                    name,
                    writable: false,
                });
            }
        }
        entries.sort_by(|a, b| {
            a.scope
                .cmp(&b.scope)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(entries)
    }

    pub fn set_enabled(entry: &StartupEntry, enabled: bool) -> Result<String, String> {
        // Validate public input before any registry access, regardless of the UI.
        validate_mutation(entry, &entry.command)?;
        let view = if entry.scope == "HKCU Run (64-bit)" {
            VIEW64
        } else {
            VIEW32
        };
        let run = open(HKCU, RUN, QUERY | view)?
            .ok_or("Run key disappeared; refresh the startup list")?;
        let current =
            query(&run, &entry.name)?.ok_or("Run value disappeared; refresh the startup list")?;
        validate_mutation(entry, &command(current.0, &current.1)?)?;
        let path = format!(r"{APPROVED}\Run");
        let existing = open(HKCU, &path, QUERY | VIEW64)?;
        let old = match &existing {
            Some(key) => query(key, &entry.name)?,
            None => None,
        };
        if old.as_ref().is_some_and(|(kind, _)| *kind != REG_BINARY) {
            return Err("StartupApproved value is not binary; use Windows Startup apps".into());
        }
        let payload = approval_payload(old.as_ref().map(|(_, bytes)| bytes.as_slice()), enabled)?;
        let path_wide = wide(&path);
        let mut key = ptr::null_mut();
        // SAFETY: fixed HKCU approval path, terminated string and writable result.
        // No Run key, Run command, HKLM value or folder content is ever written.
        let status = unsafe {
            RegCreateKeyExW(
                HKCU,
                path_wide.as_ptr(),
                0,
                ptr::null_mut(),
                0,
                QUERY | SET | VIEW64,
                ptr::null(),
                &mut key,
                ptr::null_mut(),
            )
        };
        if status != 0 {
            return Err(error(
                "Open/create current-user StartupApproved key",
                status,
            ));
        }
        let key = Key(key);
        // Recheck after opening the destination to narrow the external edit race.
        let current =
            query(&run, &entry.name)?.ok_or("Run value disappeared; refresh the startup list")?;
        validate_mutation(entry, &command(current.0, &current.1)?)?;
        let latest = query(&key, &entry.name)?;
        if latest != old {
            return Err("Startup approval changed; refresh before trying again".into());
        }
        let name = wide(&entry.name);
        // SAFETY: owned key with SET rights, terminated name, exact 12-byte payload.
        let status = unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                REG_BINARY,
                payload.as_ptr(),
                payload.len() as u32,
            )
        };
        if status != 0 {
            return Err(error("Write current-user startup approval", status));
        }
        let actual = query(&key, &entry.name)?;
        if actual != Some((REG_BINARY, payload.to_vec())) {
            return Err(
                "Startup approval changed after write; refresh to inspect its current state".into(),
            );
        }
        Ok(format!(
            "Startup approval {} for {}. Run command retained.",
            if enabled { "enabled" } else { "disabled" },
            entry.name
        ))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn registry_command_decoding_rejects_wrong_type_bad_utf16_and_embedded_nul() {
            let encode = |s: &str| {
                wide(s)
                    .into_iter()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                command(REG_EXPAND_SZ, &encode("%LOCALAPPDATA%\\app.exe")).unwrap(),
                "%LOCALAPPDATA%\\app.exe"
            );
            assert!(command(REG_BINARY, &encode("app.exe")).is_err());
            assert!(command(REG_SZ, &[0]).is_err());
            assert!(command(REG_SZ, &[0, 0xd8]).is_err());
            assert!(command(REG_SZ, &encode("app\0hidden")).is_err());
            assert!(command(REG_SZ, &encode("")).is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry() -> StartupEntry {
        StartupEntry {
            scope: "HKCU Run (64-bit)".into(),
            name: "Example".into(),
            command: r#""C:\Program Files\Example\app.exe" --quiet"#.into(),
            enabled: Some(true),
            writable: true,
        }
    }
    #[test]
    fn payload_is_twelve_bytes_with_requested_state_and_zero_timestamp() {
        for (enabled, code) in [(true, 2), (false, 3)] {
            let bytes = approval_payload(None, enabled).unwrap();
            assert_eq!(bytes.len(), 12);
            assert_eq!(bytes[0], code);
            assert!(bytes[1..].iter().all(|x| *x == 0));
            assert_eq!(approval_state(&bytes), Some(enabled));
        }
    }
    #[test]
    fn known_states_are_decoded_without_reinterpreting_unknown_payloads() {
        for (state, expected) in [(2, true), (3, false), (6, true), (7, false)] {
            let mut bytes = [0; 12];
            bytes[0] = state;
            bytes[4] = 42;
            assert_eq!(approval_state(&bytes), Some(expected));
            assert!(approval_payload(Some(&bytes), !expected).is_ok());
        }
        for length in [0, 1, 4, 11, 13, 20] {
            let bytes = vec![2; length];
            assert_eq!(approval_state(&bytes), None);
            assert!(approval_payload(Some(&bytes), true).is_err());
        }
        for bytes in [[0; 12], [255; 12], [2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]] {
            assert_eq!(approval_state(&bytes), None);
            assert!(approval_payload(Some(&bytes), false).is_err());
        }
    }
    #[test]
    fn mutation_checks_scope_capability_and_exact_command() {
        let original = entry();
        validate_mutation(&original, &original.command).unwrap();
        assert!(validate_mutation(&original, "different.exe").is_err());
        for scope in [
            "HKLM Run (64-bit)",
            "HKCU",
            "Current user Startup folder",
            "HKCU Run (64-bit)\0",
            "HKCU Run (64-bit)\\Other",
        ] {
            let altered = StartupEntry {
                scope: scope.into(),
                ..entry()
            };
            assert!(validate_mutation(&altered, &altered.command).is_err());
        }
        let readonly = StartupEntry {
            writable: false,
            ..entry()
        };
        assert!(validate_mutation(&readonly, &readonly.command).is_err());
    }
    #[test]
    fn mutation_rejects_empty_nul_and_overlong_names_or_commands() {
        for invalid in [
            String::new(),
            "bad\0value".into(),
            "x".repeat(MAX_TEXT_UNITS + 1),
        ] {
            let bad_name = StartupEntry {
                name: invalid.clone(),
                ..entry()
            };
            assert!(validate_mutation(&bad_name, &bad_name.command).is_err());
            let bad_command = StartupEntry {
                command: invalid,
                ..entry()
            };
            assert!(validate_mutation(&bad_command, &bad_command.command).is_err());
        }
        assert!(validate_text("ทดสอบ 🚀", "name").is_ok());
    }
}
