//! Read-only native telemetry collected away from the GUI thread.

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ProcessRow {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    /// Bytes read since the previous process refresh; includes non-disk I/O on Windows.
    pub read_bytes: u64,
    /// Bytes written since the previous process refresh; includes non-disk I/O on Windows.
    pub written_bytes: u64,
    pub start_time: u64,
    pub executable: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct GpuSnapshot {
    pub name: String,
    pub utilization_percent: Option<u32>,
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub temperature_c: Option<u32>,
    pub power_w: Option<f32>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub unix_seconds: u64,
    pub cpu_percent: f32,
    pub cpu_count: usize,
    pub used_memory: u64,
    pub total_memory: u64,
    pub available_memory: u64,
    pub used_swap: u64,
    pub total_swap: u64,
    pub processes: Vec<ProcessRow>,
    pub gpu: Option<GpuSnapshot>,
    pub gpu_error: Option<String>,
    pub collection_ms: u64,
}

use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nvml_wrapper::{Nvml, enum_wrappers::device::TemperatureSensor};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// A read-only collector with one consumable snapshot slot. New snapshots replace
/// unread ones; no history or growing queue is retained in this module.
///
/// CPU percentages represent total machine capacity: 100% means all logical CPUs
/// are busy. sysinfo process usage can reach 100% * logical CPUs, so it is divided
/// by that count here. One fully busy thread on 8 logical CPUs is about 12.5%.
/// Process I/O fields are interval deltas since the previous process refresh,
/// not lifetime totals or bytes/second. Windows includes non-disk I/O. A process's
/// first refresh can include its lifetime I/O because no prior baseline exists.
/// Pauses extend the measurement interval; replaced snapshots are not accumulated.
/// Protected process fields may be zero or absent. The first published snapshot
/// waits for a CPU measurement baseline.
pub struct TelemetryWorker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

struct Shared {
    controls: Mutex<Controls>,
    wake: Condvar,
    latest: Mutex<Option<Snapshot>>,
}

struct Controls {
    stopped: bool,
    paused: bool,
    interval: Duration,
}

fn minimum_interval() -> Duration {
    sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(250))
}

impl TelemetryWorker {
    /// Start a dedicated collector at a default interval of one second.
    pub fn spawn() -> Self {
        let shared = Arc::new(Shared {
            controls: Mutex::new(Controls {
                stopped: false,
                paused: false,
                interval: Duration::from_secs(1),
            }),
            wake: Condvar::new(),
            latest: Mutex::new(None),
        });
        let worker_shared = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("vanta-telemetry".into())
            .spawn(move || collect_loop(worker_shared))
            .expect("could not spawn telemetry worker");
        Self {
            shared,
            thread: Some(thread),
        }
    }

    /// Take the newest unread snapshot. Returns immediately on lock contention;
    /// the GUI can poll again on its next frame. A second read returns None until
    /// collection publishes another snapshot.
    pub fn latest(&self) -> Option<Snapshot> {
        self.shared.latest.try_lock().ok()?.take()
    }

    /// Set spacing between collections (after the preceding collection finishes).
    /// Clamped to the CPU sampling minimum (at least 250ms) through one day.
    /// Wakes the worker so a shorter interval takes effect during an existing wait.
    pub fn set_interval(&self, interval: Duration) {
        let mut controls = self
            .shared
            .controls
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        controls.interval = interval.clamp(minimum_interval(), Duration::from_secs(86_400));
        self.shared.wake.notify_one();
    }

    /// Suspend collection. An in-flight OS/driver query may finish, but its result
    /// is discarded while paused. Pausing clears the unread slot. Resume wakes
    /// the worker. CPU sampling after a
    /// pause spans the pause as well as the active measurement interval.
    pub fn set_paused(&self, paused: bool) {
        let mut controls = self
            .shared
            .controls
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        controls.paused = paused;
        let previous = if paused {
            self.shared
                .latest
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
        } else {
            None
        };
        self.shared.wake.notify_one();
        drop(controls);
        drop(previous);
    }
}

impl Default for TelemetryWorker {
    fn default() -> Self {
        Self::spawn()
    }
}

impl Drop for TelemetryWorker {
    fn drop(&mut self) {
        {
            let mut controls = self
                .shared
                .controls
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            controls.stopped = true;
            self.shared.wake.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            // A panicked collector must not cause a second panic during shutdown.
            let _ = thread.join();
        }
    }
}

fn refresh(system: &mut System) {
    system.refresh_cpu_usage();
    system.refresh_memory();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true, // Remove exited processes so the slot never retains stale rows.
        ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .with_disk_usage()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .without_tasks(),
    );
}

fn collect_loop(shared: Arc<Shared>) {
    // Dynamic loading is attempted once, including on machines without NVIDIA.
    // Keep this handle alive until the worker exits, never reinitialize per tick.
    let nvml = crate::drivers::init_nvml();
    let mut system = System::new();
    refresh(&mut system);
    let mut last_finished = Instant::now();
    let mut first = true;
    loop {
        let mut controls = shared.controls.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if controls.stopped {
                return;
            }
            if controls.paused {
                controls = shared
                    .wake
                    .wait(controls)
                    .unwrap_or_else(|e| e.into_inner());
                continue;
            }
            let interval = if first {
                minimum_interval()
            } else {
                controls.interval
            };
            let remaining = interval.saturating_sub(last_finished.elapsed());
            if remaining.is_zero() {
                break;
            }
            let (next, _) = shared
                .wake
                .wait_timeout(controls, remaining)
                .unwrap_or_else(|e| e.into_inner());
            controls = next;
        }
        drop(controls);

        let started = Instant::now();
        refresh(&mut system);
        let snapshot = make_snapshot(&system, &nvml, started);
        last_finished = Instant::now();
        first = false;

        let controls = shared.controls.lock().unwrap_or_else(|e| e.into_inner());
        if controls.stopped {
            return;
        }
        if !controls.paused {
            // Only a fixed-size Snapshot move occurs under the slot lock.
            // Drop the previous (potentially large) process vector after unlock.
            let previous = shared
                .latest
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .replace(snapshot);
            drop(controls);
            drop(previous);
        }
    }
}

fn make_snapshot(system: &System, nvml: &Result<Nvml, String>, started: Instant) -> Snapshot {
    let cpu_count = system.cpus().len().max(1);
    let mut processes: Vec<_> = system
        .processes()
        .iter()
        .map(|(pid, process)| {
            let io = process.disk_usage();
            ProcessRow {
                pid: pid.as_u32(),
                name: process.name().to_string_lossy().into_owned(),
                cpu_percent: (process.cpu_usage() / cpu_count as f32).clamp(0.0, 100.0),
                memory_bytes: process.memory(),
                read_bytes: io.read_bytes,
                written_bytes: io.written_bytes,
                start_time: process.start_time(),
                executable: process
                    .exe()
                    .map(|path| path.to_string_lossy().into_owned()),
            }
        })
        .collect();
    processes.sort_unstable_by_key(|process| process.pid);
    let (gpu, gpu_error) = match nvml {
        Ok(nvml) => match collect_gpu(nvml) {
            Ok(snapshot) => (Some(snapshot), None),
            Err(error) => (None, Some(error)),
        },
        Err(error) => (None, Some(error.clone())),
    };
    let (total_memory, available_memory, used_memory) = physical_memory(system);
    Snapshot {
        unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        cpu_percent: system.global_cpu_usage().clamp(0.0, 100.0),
        cpu_count,
        used_memory,
        total_memory,
        available_memory,
        used_swap: system.used_swap(),
        total_swap: system.total_swap(),
        processes,
        gpu,
        gpu_error,
        collection_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    }
}

fn collect_gpu(nvml: &Nvml) -> Result<GpuSnapshot, String> {
    // The public snapshot describes the first NVIDIA device, not an aggregate.
    let device = nvml
        .device_by_index(0)
        .map_err(|e| format!("NVML device 0: {e}"))?;
    let name = device
        .name()
        .map_err(|e| format!("NVML device name: {e}"))?;
    let memory = device
        .memory_info()
        .map_err(|e| format!("NVML memory: {e}"))?;
    Ok(GpuSnapshot {
        name,
        utilization_percent: device.utilization_rates().ok().map(|usage| usage.gpu),
        used_bytes: memory.used,
        total_bytes: memory.total,
        temperature_c: device.temperature(TemperatureSensor::Gpu).ok(),
        power_w: device
            .power_usage()
            .ok()
            .map(|milliwatts| milliwatts as f32 / 1000.0),
    })
}

fn physical_memory(system: &System) -> (u64, u64, u64) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::ProcessStatus::{
            GetPerformanceInfo, PERFORMANCE_INFORMATION,
        };
        let mut info = PERFORMANCE_INFORMATION {
            cb: std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32,
            ..Default::default()
        };
        // SAFETY: info is initialized, writable, correctly aligned, and its size
        // matches cb. GetPerformanceInfo retains no pointer after returning.
        if unsafe { GetPerformanceInfo(&mut info, std::mem::size_of_val(&info) as u32) } != 0 {
            let total = (info.PhysicalTotal as u64).saturating_mul(info.PageSize as u64);
            let available = (info.PhysicalAvailable as u64)
                .saturating_mul(info.PageSize as u64)
                .min(total);
            if total > 0 {
                return (total, available, total - available);
            }
        }
    }
    (
        system.total_memory(),
        system.available_memory(),
        system.used_memory(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exercise real collection through the same interface used by the GUI.
    fn wait_for_snapshot(worker: &TelemetryWorker) -> Snapshot {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Some(snapshot) = worker.latest() {
                return snapshot;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "worker did not publish"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn worker_collects_serializable_machine_and_current_process_metrics() {
        let worker = TelemetryWorker::spawn();
        worker.set_interval(std::time::Duration::from_millis(250));
        let snapshot = wait_for_snapshot(&worker);
        assert!(snapshot.cpu_count > 0);
        assert!((0.0..=100.0).contains(&snapshot.cpu_percent));
        assert!(snapshot.total_memory > 0);
        assert!(snapshot.available_memory <= snapshot.total_memory);
        assert!(snapshot.used_memory <= snapshot.total_memory);
        let own_process = snapshot
            .processes
            .iter()
            .find(|row| row.pid == std::process::id())
            .expect("current process missing");
        assert!(!own_process.name.is_empty());
        assert!((0.0..=100.0).contains(&own_process.cpu_percent));
        assert!(snapshot.gpu.is_some() || snapshot.gpu_error.is_some());
        let json = serde_json::to_string(&snapshot).unwrap();
        let decoded: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.unix_seconds, snapshot.unix_seconds);
    }
    #[test]
    fn pause_discards_unread_snapshots_and_resume_wakes_long_wait() {
        let worker = TelemetryWorker::spawn();
        worker.set_interval(Duration::from_millis(250));
        let _ = wait_for_snapshot(&worker);
        // Let at least one subsequent snapshot accumulate unread.
        thread::sleep(Duration::from_secs(1));
        worker.set_paused(true);
        assert!(
            worker.latest().is_none(),
            "pause must discard the pending snapshot"
        );
        thread::sleep(Duration::from_millis(400));
        assert!(
            worker.latest().is_none(),
            "paused worker published a snapshot"
        );
        worker.set_interval(Duration::from_secs(86_400));
        worker.set_paused(false);
        worker.set_interval(Duration::from_millis(250));
        let resumed = wait_for_snapshot(&worker);
        assert!(resumed.total_memory > 0);
        worker.set_paused(true);
        let started = Instant::now();
        drop(worker);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "paused shutdown did not wake"
        );
    }
    #[test]
    fn zero_interval_is_bounded_and_unread_slot_keeps_the_newest_snapshot() {
        let worker = TelemetryWorker::spawn();
        worker.set_interval(Duration::ZERO);
        let first = wait_for_snapshot(&worker);
        assert!(
            worker.latest().is_none(),
            "snapshot should be consumed once"
        );
        thread::sleep(Duration::from_millis(80));
        assert!(
            worker.latest().is_none(),
            "zero interval bypassed minimum sampling time"
        );
        thread::sleep(Duration::from_millis(2200));
        worker.set_interval(Duration::MAX);
        // Allow any in-flight collection to finish before consuming the slot.
        thread::sleep(Duration::from_millis(500));
        let newest = worker.latest().expect("unread snapshot missing");
        assert!(
            newest.unix_seconds > first.unix_seconds,
            "an old queued snapshot was returned instead of the newest"
        );
        assert!(
            worker.latest().is_none(),
            "more than one snapshot was queued"
        );
        let started = Instant::now();
        drop(worker);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "long-interval shutdown did not wake"
        );
    }
    #[cfg(windows)]
    #[test]
    fn process_io_reports_bytes_since_previous_refresh_instead_of_lifetime_totals() {
        use std::io::Read;

        // Windows accounts cached reads too, so reading the existing test binary
        // exercises I/O counters without creating files or changing a process.
        let read_existing_binary = || {
            let mut file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
            file.read_exact(&mut [0_u8; 4096]).unwrap();
        };
        read_existing_binary();
        let pid = sysinfo::Pid::from_u32(std::process::id());
        let mut system = System::new();
        let refresh_io = |system: &mut System| {
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                true,
                ProcessRefreshKind::nothing().with_disk_usage(),
            );
        };
        refresh_io(&mut system);
        let previous = system.process(pid).unwrap().disk_usage();
        assert!(
            previous.total_read_bytes >= 4096,
            "missing baseline read I/O"
        );

        read_existing_binary();
        refresh_io(&mut system);
        let current = system.process(pid).unwrap().disk_usage();
        let snapshot = make_snapshot(
            &system,
            &Err("test has no GPU collector".into()),
            Instant::now(),
        );
        let row = snapshot
            .processes
            .iter()
            .find(|row| row.pid == pid.as_u32())
            .unwrap();
        assert!(row.read_bytes >= 4096, "the latest read was not observed");
        assert_eq!(
            row.read_bytes,
            current.total_read_bytes - previous.total_read_bytes
        );
        assert_eq!(
            row.written_bytes,
            current.total_written_bytes - previous.total_written_bytes
        );
        assert!(
            row.read_bytes < current.total_read_bytes,
            "lifetime I/O leaked into interval bytes"
        );
    }
    #[test]
    fn snapshot_json_round_trip_preserves_counters_and_optional_fields() {
        let json = serde_json::json!({
            "unix_seconds": 1_800_000_000_u64, "cpu_percent": 25.5, "cpu_count": 8,
            "used_memory": 8_000_000_000_u64, "total_memory": 16_000_000_000_u64,
            "available_memory": 8_000_000_000_u64, "used_swap": 12_u64,
            "total_swap": 4_000_000_000_u64, "collection_ms": 17,
            "processes": [{ "pid": 42, "name": "ๆธฌๅฎ.exe", "cpu_percent": 12.5,
                "memory_bytes": 4_500_000_000_u64, "read_bytes": 9_000_000_001_u64,
                "written_bytes": 8_000_000_001_u64, "start_time": 1_700_000_000_u64,
                "executable": null }],
            "gpu": { "name": "NVIDIA GPU", "utilization_percent": null,
                "used_bytes": 5_000_000_000_u64, "total_bytes": 8_000_000_000_u64,
                "temperature_c": 65, "power_w": 42.5 },
            "gpu_error": null
        });
        let snapshot: Snapshot = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(snapshot.clone()).unwrap(), json);
        assert_eq!(snapshot.processes[0].read_bytes, 9_000_000_001);
        assert!(snapshot.processes[0].executable.is_none());
        assert!(snapshot.gpu.unwrap().utilization_percent.is_none());
    }
}
