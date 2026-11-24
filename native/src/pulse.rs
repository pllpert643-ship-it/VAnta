//! Bounded GPU memory pulses using the CUDA driver installed with NVIDIA's driver.

/// Accept only 32..=2048 MiB and 1..=3600 seconds, without touching the GPU.
pub fn validate(memory_mb: u64, duration_seconds: u64) -> Result<(), String> {
    if !(32..=2048).contains(&memory_mb) {
        return Err("memory_mb must be between 32 and 2048 MiB".into());
    }
    if !(1..=3600).contains(&duration_seconds) {
        return Err("duration_seconds must be between 1 and 3600 seconds".into());
    }
    Ok(())
}

/// Run a bounded CUDA memory pulse on device zero. No CUDA toolkit is required.
/// Validation precedes driver loading; all acquired resources are released on exit.
pub fn run(memory_mb: u64, duration_seconds: u64) -> Result<(), String> {
    validate(memory_mb, duration_seconds)?;
    #[cfg(windows)]
    {
        windows::run(memory_mb, duration_seconds)
    }
    #[cfg(not(windows))]
    {
        Err("GPU Pulse is only supported on Windows".into())
    }
}

#[cfg(windows)]
mod windows {
    use libloading::os::windows::{LOAD_LIBRARY_SEARCH_SYSTEM32, Library};
    use std::ffi::c_void;
    use std::time::{Duration, Instant};

    type Context = *mut c_void;
    type Init = unsafe extern "system" fn(u32) -> i32;
    type DeviceGet = unsafe extern "system" fn(*mut i32, i32) -> i32;
    type ContextCreate = unsafe extern "system" fn(*mut Context, u32, i32) -> i32;
    type MemAlloc = unsafe extern "system" fn(*mut u64, usize) -> i32;
    type Memset = unsafe extern "system" fn(u64, u8, usize) -> i32;
    type Synchronize = unsafe extern "system" fn() -> i32;
    type MemFree = unsafe extern "system" fn(u64) -> i32;
    type ContextDestroy = unsafe extern "system" fn(Context) -> i32;

    fn check(operation: &str, result: i32) -> Result<(), String> {
        if result == 0 {
            Ok(())
        } else {
            Err(format!("{operation} failed with CUDA error {result}"))
        }
    }

    fn load_library(filename: &str) -> Result<Library, String> {
        let path = crate::drivers::system_library(filename)?;
        // SAFETY: production passes only the NVIDIA driver filename. The absolute
        // system path and SYSTEM32 search flag exclude cwd for it and dependencies.
        unsafe { Library::load_with_flags(&path, LOAD_LIBRARY_SEARCH_SYSTEM32) }
            .map_err(|error| format!("Cannot load CUDA driver {}: {error}", path.display()))
    }

    struct Driver {
        init: Init,
        device_get: DeviceGet,
        context_create: ContextCreate,
        mem_alloc: MemAlloc,
        memset: Memset,
        synchronize: Synchronize,
        mem_free: MemFree,
        context_destroy: ContextDestroy,
        // Keep every function pointer valid through resource cleanup.
        _library: Library,
    }

    impl Driver {
        fn load() -> Result<Self, String> {
            let library = load_library("nvcuda.dll")?;
            // SAFETY: names and types match the CUDA Driver ABI. All symbols are
            // resolved before initialization, and the library lives with Driver.
            unsafe {
                Ok(Self {
                    init: Self::symbol(&library, b"cuInit\0")?,
                    device_get: Self::symbol(&library, b"cuDeviceGet\0")?,
                    context_create: Self::symbol(&library, b"cuCtxCreate_v2\0")?,
                    mem_alloc: Self::symbol(&library, b"cuMemAlloc_v2\0")?,
                    memset: Self::symbol(&library, b"cuMemsetD8_v2\0")?,
                    synchronize: Self::symbol(&library, b"cuCtxSynchronize\0")?,
                    mem_free: Self::symbol(&library, b"cuMemFree_v2\0")?,
                    context_destroy: Self::symbol(&library, b"cuCtxDestroy_v2\0")?,
                    _library: library,
                })
            }
        }

        // SAFETY: caller must supply the exact function type for the symbol.
        unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, String> {
            unsafe { library.get::<T>(name) }
                .map(|symbol| *symbol)
                .map_err(|error| {
                    format!(
                        "Missing CUDA symbol {}: {error}",
                        String::from_utf8_lossy(&name[..name.len() - 1])
                    )
                })
        }
    }

    struct Resources<'a> {
        driver: &'a Driver,
        context: Option<Context>,
        allocation: Option<u64>,
    }

    impl Resources<'_> {
        fn close(&mut self) -> Result<(), String> {
            let mut errors = Vec::new();
            if let Some(pointer) = self.allocation.take() {
                // SAFETY: this allocation belongs to our still-current context.
                if let Err(error) =
                    check("cuMemFree_v2", unsafe { (self.driver.mem_free)(pointer) })
                {
                    errors.push(error);
                }
            }
            if let Some(context) = self.context.take() {
                // SAFETY: context is owned by this guard; destruction is attempted
                // even when freeing memory fails (it also releases context memory).
                if let Err(error) = check("cuCtxDestroy_v2", unsafe {
                    (self.driver.context_destroy)(context)
                }) {
                    errors.push(error);
                }
            }
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors.join("; "))
            }
        }
    }

    impl Drop for Resources<'_> {
        fn drop(&mut self) {
            // Normal Result paths explicitly close and return cleanup errors.
            // Drop also cleans up during unwinding, without panicking a second time.
            if let Err(error) = self.close() {
                use std::io::Write;
                let _ = writeln!(std::io::stderr(), "GPU Pulse cleanup failed: {error}");
            }
        }
    }

    pub(super) fn run(memory_mb: u64, duration_seconds: u64) -> Result<(), String> {
        let driver = Driver::load()?;
        let mut resources = Resources {
            driver: &driver,
            context: None,
            allocation: None,
        };
        let result = (|| {
            // SAFETY: all calls use checked CUDA ABI symbols. Resources stay on
            // this thread, where context creation makes the context current.
            unsafe {
                check("cuInit", (driver.init)(0))?;
                let mut device = 0;
                check("cuDeviceGet", (driver.device_get)(&mut device, 0))?;
                let mut context = std::ptr::null_mut();
                check(
                    "cuCtxCreate_v2",
                    (driver.context_create)(&mut context, 0, device),
                )?;
                resources.context = Some(context);
                let bytes = usize::try_from(memory_mb * 1024 * 1024)
                    .map_err(|_| "GPU allocation size exceeds address space".to_string())?;
                let mut pointer = 0;
                check("cuMemAlloc_v2", (driver.mem_alloc)(&mut pointer, bytes))?;
                resources.allocation = Some(pointer);
                let started = Instant::now();
                let duration = Duration::from_secs(duration_seconds);
                let mut value = 0u8;
                while started.elapsed() < duration {
                    check("cuMemsetD8_v2", (driver.memset)(pointer, value, bytes))?;
                    check("cuCtxSynchronize", (driver.synchronize)())?;
                    value = value.wrapping_add(1);
                    std::thread::sleep(Duration::from_millis(15));
                }
            }
            Ok(())
        })();
        let cleanup = resources.close();
        match (result, cleanup) {
            (Err(error), Err(cleanup)) => Err(format!("{error}; cleanup: {cleanup}")),
            (Err(error), _) | (_, Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn unavailable_library_returns_error_without_initializing_cuda() {
            // A missing name exercises the actual system-directory loader even
            // on NVIDIA machines, without loading nvcuda or invoking GPU work.
            let result = super::load_library("vanta-test-missing-cuda-7c939572.dll");
            assert!(result.is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_rejects_invalid_arguments_before_loading_cuda() {
        for (memory, seconds) in [(31, 1), (2049, 1), (32, 0), (32, 3601)] {
            assert_eq!(run(memory, seconds), validate(memory, seconds));
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn valid_workload_is_explicitly_unsupported_outside_windows() {
        assert_eq!(
            run(32, 1),
            Err("GPU Pulse is only supported on Windows".into())
        );
    }

    #[test]
    fn duration_limits_are_inclusive() {
        for seconds in [0, 3601, u64::MAX] {
            assert!(validate(32, seconds).is_err(), "accepted {seconds} seconds");
        }
        for seconds in [1, 3600] {
            assert!(validate(32, seconds).is_ok());
        }
    }

    #[test]
    fn memory_limits_are_inclusive() {
        for memory in [0, 31, 2049, u64::MAX] {
            assert!(validate(memory, 1).is_err(), "accepted {memory} MiB");
        }
        for memory in [32, 2048] {
            assert!(validate(memory, 1).is_ok());
        }
    }
}
