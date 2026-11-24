//! Load optional vendor code from a Windows-managed directory, never the working directory.
#[cfg(windows)]
pub fn system_library(name: &str) -> Result<std::path::PathBuf, String> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    if !matches!(name, "nvml.dll" | "nvcuda.dll") {
        return Err("Unsupported optional driver library".into());
    }
    let mut buffer = vec![0u16; 32768];
    // SAFETY: valid writable UTF-16 buffer; this only queries Windows' directory.
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if length == 0 {
        return Err(format!(
            "Cannot locate Windows system directory: {}",
            std::io::Error::last_os_error()
        ));
    }
    if length >= buffer.len() {
        return Err("Windows system directory is too long".into());
    }
    let directory = PathBuf::from(OsString::from_wide(&buffer[..length]));
    if !directory.is_absolute() {
        return Err("Windows system directory is not absolute".into());
    }
    Ok(directory.join(name))
}

pub fn init_nvml() -> Result<nvml_wrapper::Nvml, String> {
    #[cfg(windows)]
    let result = {
        let path = system_library("nvml.dll")?;
        nvml_wrapper::Nvml::builder()
            .lib_path(path.as_os_str())
            .init()
    };
    #[cfg(not(windows))]
    let result = nvml_wrapper::Nvml::init();
    result.map_err(|e| format!("NVML unavailable: {e}"))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn vendor_library_location_cannot_be_redirected_to_a_relative_or_arbitrary_dll() {
        for name in ["nvml.dll", "nvcuda.dll"] {
            let path = system_library(name).unwrap();
            assert!(path.is_absolute());
            assert_eq!(path.file_name().unwrap(), name);
            assert!(
                path.parent()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("system32")
            );
        }
        for name in ["../nvml.dll", "C:\\temp\\nvml.dll", "other.dll", ""] {
            assert!(system_library(name).is_err());
        }
    }
}
