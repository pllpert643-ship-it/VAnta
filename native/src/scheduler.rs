//! Serial GPU admission and durable native-only queue storage.
//!
//! Pass a native-specific data directory. Recovery always pauses dispatch until
//! `set_paused(false)`. Persisted PIDs are informational and never acted upon.
//! Windows children start suspended, are assigned to a private kill-on-close
//! Job Object, and resume only after membership is verified and log readers exist.
//! Assignment/resume failure cancels the still-owned child before dispatch continues.
//! Rust 1.100's stable main_thread_handle preserves the actual primary thread handle.
//! Non-Windows controls direct children only.
//! Log readers never block the UI on joins; Windows readers stop after bounded draining.
//! Logs retain at most 1 MiB per stream per retained job; excess is drained.
//! Post-launch observations compare global VRAM on the same fixed device (NVML
//! index 0). Callers pass only fresh budgets, or None when telemetry is stale.
//! Unmonitored launches have no baseline and produce no delta observation.
//! Call shutdown explicitly to observe cleanup errors before dropping the module.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
};

const RESERVE_MB: u64 = 512;
const SELF_EXECUTABLE: &str = "vanta-native://self";
const MAX_EVENTS: usize = 256;
const MAX_HISTORY: usize = 128;
const MAX_PENDING: usize = 1024;
const MAX_LOG: usize = 1024 * 1024;
const MAX_STORE: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    Critical,
    High,
    Normal,
    Low,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Queued,
    Waiting,
    Running,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub executable: String,
    pub args: Vec<String>,
    pub priority: Priority,
    pub required_vram_mb: u64,
    pub status: Status,
    pub reason: String,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
}
#[derive(Clone, Copy, Debug)]
pub struct GpuBudget {
    pub total_mb: u64,
    pub used_mb: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u32,
    next_id: u64,
    jobs: Vec<Job>,
    events: Vec<String>,
}
struct RunningObservation {
    baseline: GpuBudget,
    started: std::time::Instant,
    verified: bool,
}
struct OwnedChild {
    id: u64,
    child: Child,
    observation: Option<RunningObservation>,
    #[cfg(windows)]
    containment: WindowsJob,
}
struct LogPump {
    handle: JoinHandle<()>,
    errors: Arc<Mutex<Vec<String>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}
pub struct Scheduler {
    pub jobs: Vec<Job>,
    pub events: Vec<String>,
    pub paused: bool,
    pub allow_unmonitored: bool,
    pub last_error: Option<String>,
    data_dir: PathBuf,
    next_id: u64,
    blocked: bool,
    recovery_required: bool,
    owned: Option<OwnedChild>,
    pumps: Vec<LogPump>,
}
impl Scheduler {
    pub fn new(data_dir: PathBuf) -> Self {
        let mut scheduler = Self {
            jobs: vec![],
            events: vec![],
            paused: false,
            allow_unmonitored: false,
            last_error: None,
            data_dir,
            next_id: 1,
            blocked: false,
            recovery_required: false,
            owned: None,
            pumps: vec![],
        };
        if let Err(error) = scheduler.load() {
            scheduler.blocked = true;
            scheduler.paused = true;
            scheduler.error(error);
        }
        scheduler
    }
    fn load(&mut self) -> Result<(), String> {
        fs::create_dir_all(&self.data_dir)
            .map_err(|e| format!("Create native data directory: {e}"))?;
        let path = self.data_dir.join("scheduler.json");
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("Read {}: {e}", path.display())),
        };
        let mut bytes = vec![];
        file.take(MAX_STORE + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Read queue: {e}"))?;
        if bytes.len() as u64 > MAX_STORE {
            return Err("Queue file exceeds storage limit; original preserved".into());
        }
        let stored: Stored = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Corrupt queue {}; original preserved: {e}", path.display()))?;
        let mut ids = std::collections::HashSet::new();
        if stored.version != 1
            || stored.next_id == 0
            || stored
                .jobs
                .iter()
                .any(|j| j.id == 0 || j.id >= stored.next_id || !ids.insert(j.id))
        {
            return Err("Invalid queue version or job IDs; original preserved".into());
        }
        self.next_id = stored.next_id;
        self.jobs = stored.jobs;
        self.events = stored.events;
        self.paused = true;
        self.recovery_required = true;
        for job in &mut self.jobs {
            job.pid = None;
            if job.status == Status::Running {
                job.status = Status::Interrupted;
                job.reason =
                    "Controller interrupted; previous process ownership cannot be recovered".into();
            }
        }
        self.event("Recovered queue; explicit resume required".into());
        self.save()
    }
    /// Queue a command supplied by this application across package-path changes.
    pub fn submit_self(
        &mut self,
        name: String,
        args: Vec<String>,
        priority: Priority,
        required_vram_mb: u64,
    ) -> Result<u64, String> {
        self.submit(
            name,
            SELF_EXECUTABLE.into(),
            args,
            priority,
            required_vram_mb,
        )
    }
    pub fn submit(
        &mut self,
        name: String,
        executable: String,
        args: Vec<String>,
        priority: Priority,
        required_vram_mb: u64,
    ) -> Result<u64, String> {
        let result = self.submit_inner(name, executable, args, priority, required_vram_mb);
        if let Err(error) = &result {
            self.error(error.clone());
        }
        result
    }
    fn submit_inner(
        &mut self,
        name: String,
        executable: String,
        args: Vec<String>,
        priority: Priority,
        required_vram_mb: u64,
    ) -> Result<u64, String> {
        if self.blocked {
            return Err(
                "Storage blocked; inspect last_error and preserve original queue before reopening"
                    .into(),
            );
        }
        if name.trim().is_empty()
            || executable.trim().is_empty()
            || executable.contains('\0')
            || args.iter().any(|a| a.contains('\0'))
        {
            return Err("Provide a name, executable and valid argv strings".into());
        }
        // Bound both queue storage and the immutable input text per job.
        if name.len() > 1024
            || executable.len() > 32768
            || args.iter().map(String::len).sum::<usize>() > 65536
            || args.len() > 4096
        {
            return Err("Job text exceeds storage limits".into());
        }
        if required_vram_mb.checked_add(RESERVE_MB).is_none() {
            return Err("VRAM request overflows safe capacity".into());
        }
        if self.jobs.iter().filter(|j| pending(j.status)).count() >= MAX_PENDING {
            return Err("Pending queue is full".into());
        }
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("Job ID space exhausted")?;
        self.jobs.push(Job {
            id,
            name,
            executable,
            args,
            priority,
            required_vram_mb,
            status: Status::Queued,
            reason: "Waiting for scheduler".into(),
            pid: None,
            exit_code: None,
        });
        self.event(format!("Queued job {id}"));
        if let Err(error) = self.save() {
            self.jobs.retain(|j| j.id != id);
            return Err(error);
        }
        Ok(id)
    }
    pub fn tick(&mut self, budget: Option<GpuBudget>) {
        self.collect_logs();
        self.poll();
        self.observe_running(budget);
        if self.paused
            || self.recovery_required
            || self.blocked
            || self.owned.is_some()
            || !self.pumps.is_empty()
        {
            return;
        }
        loop {
            let Some(index) = self
                .jobs
                .iter()
                .enumerate()
                .filter(|(_, j)| pending(j.status))
                .min_by_key(|(_, j)| (rank(j.priority), j.id))
                .map(|(i, _)| i)
            else {
                return;
            };
            let required = self.jobs[index].required_vram_mb;
            let wait = match budget {
                None if !self.allow_unmonitored => {
                    Some("Waiting for GPU telemetry (fail closed)".to_string())
                }
                Some(b) if b.total_mb == 0 || b.used_mb > b.total_mb => {
                    Some("Waiting for valid GPU telemetry".into())
                }
                Some(b)
                    if required > b.total_mb.saturating_sub(RESERVE_MB)
                        || b.total_mb < RESERVE_MB =>
                {
                    self.transition(
                        index,
                        Status::Failed,
                        "Request exceeds total GPU capacity after 512 MiB reserve".into(),
                    );
                    if !self.persist() {
                        return;
                    }
                    continue;
                }
                Some(b) if b.total_mb - b.used_mb < required + RESERVE_MB => {
                    Some("Waiting for free VRAM including 512 MiB reserve".into())
                }
                _ => None,
            };
            if let Some(reason) = wait {
                if self.jobs[index].status != Status::Waiting || self.jobs[index].reason != reason {
                    self.transition(index, Status::Waiting, reason);
                    self.persist();
                }
                return;
            }
            // Persist admission intent first: a crash during spawn recovers as Interrupted.
            self.transition(
                index,
                Status::Running,
                "Admitted by strict priority GPU policy".into(),
            );
            if !self.persist() {
                return;
            }
            match self.start(index) {
                Ok(()) => {
                    if let Some(owned) = &mut self.owned {
                        owned.observation = budget.map(|baseline| RunningObservation {
                            baseline,
                            started: std::time::Instant::now(),
                            verified: false,
                        });
                    }
                    self.persist();
                    return;
                }
                Err(error) => {
                    self.error(error.clone());
                    if self.owned.is_some() {
                        return;
                    }
                    self.transition(index, Status::Failed, error);
                    if !self.persist() {
                        return;
                    }
                }
            }
        }
    }
    fn observe_running(&mut self, budget: Option<GpuBudget>) {
        let Some(budget) = budget else {
            return;
        };
        let Some(owned) = &mut self.owned else {
            return;
        };
        let Some(observation) = &mut owned.observation else {
            return;
        };
        if observation.verified
            || observation.started.elapsed() < std::time::Duration::from_secs(2)
            || budget.total_mb == 0
            || budget.used_mb > budget.total_mb
            || budget.total_mb != observation.baseline.total_mb
            || !matches!(owned.child.try_wait(), Ok(None))
        {
            return;
        }
        let delta = i128::from(budget.used_mb) - i128::from(observation.baseline.used_mb);
        let pid = owned.child.id();
        observation.verified = true;
        self.event(format!("Observed global VRAM delta {delta:+} MB while PID {pid} running; not attributable per-process allocation"));
        self.persist();
    }
    fn start(&mut self, index: usize) -> Result<(), String> {
        let job = &self.jobs[index];
        let log_dir = self.data_dir.join("logs");
        fs::create_dir_all(&log_dir).map_err(|e| format!("Create logs: {e}"))?;
        let stdout = File::create(log_dir.join(format!("{}-stdout.log", job.id)))
            .map_err(|e| format!("Create stdout log: {e}"))?;
        let stderr = File::create(log_dir.join(format!("{}-stderr.log", job.id)))
            .map_err(|e| format!("Create stderr log: {e}"))?;
        let executable = if job.executable == SELF_EXECUTABLE {
            std::env::current_exe()
                .map_err(|e| format!("Locate current built-in workload executable: {e}"))?
        } else {
            PathBuf::from(&job.executable)
        };
        let mut command = Command::new(&executable);
        command
            .args(&job.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000004); // CREATE_NO_WINDOW | CREATE_SUSPENDED
            let extension = Path::new(&job.executable)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if extension.eq_ignore_ascii_case("bat") || extension.eq_ignore_ascii_case("cmd") {
                return Err(
                    "Batch files require shell interpretation; supply a native executable".into(),
                );
            }
        }
        #[cfg(windows)]
        let containment = WindowsJob::new().map_err(|e| format!("Create job containment: {e}"))?;
        let child = command
            .spawn()
            .map_err(|e| format!("Spawn job {}: {e}", job.id))?;
        let pid = child.id();
        self.owned = Some(OwnedChild {
            id: job.id,
            child,
            observation: None,
            #[cfg(windows)]
            containment,
        });
        self.jobs[index].pid = Some(pid);
        #[cfg(windows)]
        if let Err(error) = self
            .owned
            .as_ref()
            .unwrap()
            .containment
            .assign(&self.owned.as_ref().unwrap().child)
        {
            let message = format!("Assign owned child to Job Object: {error}");
            self.cancel(self.jobs[index].id)?;
            return Err(message);
        }
        let owned = self.owned.as_mut().unwrap();
        let out = owned.child.stdout.take().unwrap();
        let err = owned.child.stderr.take().unwrap();
        let id = self.jobs[index].id;
        self.attach_log(pump(out, stdout), id)?;
        self.attach_log(pump(err, stderr), id)?;
        #[cfg(windows)]
        {
            let owned = self.owned.as_ref().unwrap();
            if let Err(error) = owned.containment.resume(&owned.child) {
                let message = format!("Resume contained workload: {error}");
                self.cancel(id)?;
                return Err(message);
            }
        }
        Ok(())
    }
    fn attach_log(&mut self, result: io::Result<LogPump>, id: u64) -> Result<(), String> {
        match result {
            Ok(pump) => {
                self.pumps.push(pump);
                Ok(())
            }
            Err(error) => {
                let message = format!("Start log reader: {error}");
                self.error(message.clone());
                self.cancel(id)?;
                Err(message)
            }
        }
    }
    fn poll(&mut self) {
        let Some(owned) = self.owned.as_mut() else {
            return;
        };
        let id = owned.id;
        match owned.child.try_wait() {
            Ok(Some(exit)) => {
                #[cfg(windows)]
                if let Err(error) = owned.containment.terminate() {
                    self.error(format!("Terminate owned descendants after exit: {error}"));
                    return;
                }
                self.owned = None;
                self.stop_logs();
                if let Some(i) = self.jobs.iter().position(|j| j.id == id) {
                    self.jobs[i].exit_code = exit.code();
                    self.jobs[i].pid = None;
                    self.transition(
                        i,
                        if exit.success() {
                            Status::Completed
                        } else {
                            Status::Failed
                        },
                        format!("Process exited: {exit}"),
                    );
                }
                self.persist();
            }
            Ok(None) => {}
            Err(error) => self.error(format!("Poll owned child {id}: {error}")),
        }
    }
    pub fn cancel(&mut self, id: u64) -> Result<(), String> {
        let result = self.cancel_inner(id);
        if let Err(error) = &result {
            self.error(error.clone());
        }
        result
    }
    fn cancel_inner(&mut self, id: u64) -> Result<(), String> {
        let index = self
            .jobs
            .iter()
            .position(|j| j.id == id)
            .ok_or("Unknown job ID")?;
        if self.owned.as_ref().is_some_and(|owned| owned.id == id) {
            let owned = self.owned.as_mut().unwrap();
            #[cfg(windows)]
            owned
                .containment
                .terminate()
                .map_err(|e| format!("Terminate owned Job Object: {e}; ownership retained"))?;
            let exit = match owned
                .child
                .try_wait()
                .map_err(|e| format!("Poll before cancellation: {e}"))?
            {
                Some(exit) => exit,
                None => {
                    if let Err(error) = owned.child.kill() {
                        // Exit can race kill. Only confirmed exit permits releasing ownership.
                        match owned.child.try_wait() {
                            Ok(Some(exit)) => exit,
                            _ => {
                                return Err(format!(
                                    "Cancel owned child {id}: {error}; ownership retained"
                                ));
                            }
                        }
                    } else {
                        owned.child.wait().map_err(|e| {
                            format!("Reap cancelled child {id}: {e}; ownership retained")
                        })?
                    }
                }
            };
            self.owned = None;
            self.stop_logs();
            self.jobs[index].exit_code = exit.code();
        } else if !pending(self.jobs[index].status) {
            return Err("Job is terminal or no longer owned".into());
        }
        self.jobs[index].pid = None;
        self.transition(index, Status::Cancelled, "Cancelled by controller".into());
        self.save()
    }
    pub fn set_paused(&mut self, paused: bool) {
        if self.blocked && !paused {
            self.error("Cannot resume while storage is blocked".into());
            return;
        }
        self.paused = paused;
        if !paused {
            self.recovery_required = false;
        }
        self.event(
            if paused {
                "Queue paused"
            } else {
                "Queue resumed explicitly"
            }
            .into(),
        );
        self.persist();
    }
    pub fn shutdown(&mut self) {
        self.paused = true;
        if let Some(id) = self.owned.as_ref().map(|owned| owned.id)
            && let Err(error) = self.cancel(id)
        {
            self.error(format!("Shutdown: {error}"));
        }
        self.collect_logs();
        self.persist();
    }
    fn transition(&mut self, index: usize, status: Status, reason: String) {
        self.jobs[index].status = status;
        self.jobs[index].reason = reason.clone();
        self.event(format!("Job {}: {status:?}: {reason}", self.jobs[index].id));
    }
    fn event(&mut self, message: String) {
        self.events.push(message.chars().take(2048).collect());
        let extra = self.events.len().saturating_sub(MAX_EVENTS);
        self.events.drain(..extra);
    }
    fn error(&mut self, message: String) {
        if !self.blocked || self.last_error.is_none() {
            self.last_error = Some(message.clone());
        }
        self.event(format!("Error: {message}"));
    }
    fn stop_logs(&self) {
        for pump in &self.pumps {
            pump.stop.store(true, std::sync::atomic::Ordering::Release);
        }
    }
    fn collect_logs(&mut self) {
        let mut errors = vec![];
        for i in (0..self.pumps.len()).rev() {
            match self.pumps[i].errors.lock() {
                Ok(mut pending) => errors.append(&mut pending),
                Err(_) => errors.push("Log error reporting mutex poisoned".into()),
            }
            if self.pumps[i].handle.is_finished() {
                let pump = self.pumps.swap_remove(i);
                if pump.handle.join().is_err() {
                    errors.push("Log reader panicked".into());
                }
                match pump.errors.lock() {
                    Ok(mut pending) => errors.append(&mut pending),
                    Err(_) => errors.push("Log error reporting mutex poisoned".into()),
                }
            }
        }
        for error in errors {
            self.error(error);
        }
    }
    fn persist(&mut self) -> bool {
        if self.blocked {
            return false;
        }
        match self.save() {
            Ok(()) => true,
            Err(error) => {
                self.error(error);
                false
            }
        }
    }
    fn save(&mut self) -> Result<(), String> {
        if self.blocked {
            return Err("Queue storage is blocked; original preserved".into());
        }
        let result = self.save_inner();
        if let Err(error) = &result {
            self.error(error.clone());
            self.blocked = true;
            self.paused = true;
        }
        result
    }
    fn save_inner(&mut self) -> Result<(), String> {
        let terminal_count = self
            .jobs
            .iter()
            .filter(|j| !pending(j.status) && j.status != Status::Running)
            .count();
        let mut remove = terminal_count.saturating_sub(MAX_HISTORY);
        let owned_id = self.owned.as_ref().map(|c| c.id);
        let mut removed = vec![];
        self.jobs.retain(|j| {
            if remove > 0
                && !pending(j.status)
                && j.status != Status::Running
                && Some(j.id) != owned_id
            {
                remove -= 1;
                removed.push(j.id);
                false
            } else {
                true
            }
        });
        for id in removed {
            for stream in ["stdout", "stderr"] {
                let path = self
                    .data_dir
                    .join("logs")
                    .join(format!("{id}-{stream}.log"));
                match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("Remove old log {}: {e}", path.display())),
                }
            }
        }
        let stored = Stored {
            version: 1,
            next_id: self.next_id,
            jobs: self.jobs.clone(),
            events: self.events.clone(),
        };
        let bytes = serde_json::to_vec(&stored).map_err(|e| format!("Serialize queue: {e}"))?;
        if bytes.len() as u64 > MAX_STORE {
            return Err("Queue exceeds storage byte limit".into());
        }
        let temporary = self.data_dir.join("scheduler.json.tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)
            .map_err(|e| format!("Open temporary queue: {e}"))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("Write/sync temporary queue: {e}"))?;
        drop(file);
        atomic_replace(&temporary, &self.data_dir.join("scheduler.json"))
            .map_err(|e| format!("Replace queue atomically: {e}"))
    }
}
impl Drop for Scheduler {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn pending(status: Status) -> bool {
    matches!(status, Status::Queued | Status::Waiting)
}
fn rank(priority: Priority) -> u8 {
    match priority {
        Priority::Critical => 0,
        Priority::High => 1,
        Priority::Normal => 2,
        Priority::Low => 3,
    }
}
fn pump(mut reader: impl LogReader, mut file: File) -> io::Result<LogPump> {
    let errors = Arc::new(Mutex::new(vec![]));
    let report = errors.clone();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopping = stop.clone();
    let handle = thread::Builder::new()
        .name("vanta-log".into())
        .spawn(move || {
            let mut retained = 0;
            let mut writable = true;
            let mut buffer = [0u8; 8192];
            let report_error = |message: String| {
                if let Ok(mut errors) = report.lock() {
                    errors.push(message);
                }
            };
            #[cfg(windows)]
            let mut drain_started = None;
            loop {
                #[cfg(windows)]
                let read_limit = {
                    let stopping = stopping.load(std::sync::atomic::Ordering::Acquire);
                    if stopping
                        && drain_started
                            .get_or_insert_with(std::time::Instant::now)
                            .elapsed()
                            >= std::time::Duration::from_millis(250)
                    {
                        break;
                    }
                    match pipe_available(&reader) {
                        Ok(0) if stopping => break,
                        Ok(0) => {
                            thread::sleep(std::time::Duration::from_millis(10));
                            continue;
                        }
                        Ok(available) => (available as usize).min(buffer.len()),
                        Err(e) if e.raw_os_error() == Some(109) => break, // ERROR_BROKEN_PIPE
                        Err(error) => {
                            report_error(format!("Inspect child output pipe: {error}"));
                            break;
                        }
                    }
                };
                #[cfg(not(windows))]
                let read_limit = {
                    let _ = &stopping;
                    buffer.len()
                };
                match reader.read(&mut buffer[..read_limit]) {
                    Ok(0) => break,
                    Ok(n) => {
                        let keep = n.min(MAX_LOG - retained);
                        if keep > 0
                            && writable
                            && let Err(error) = file.write_all(&buffer[..keep])
                        {
                            report_error(format!("Write child log: {error}"));
                            writable = false;
                        }
                        retained += keep;
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        report_error(format!("Read child output: {error}"));
                        break;
                    }
                }
            }
            if let Err(error) = file.sync_all() {
                report_error(format!("Sync child log: {error}"));
            }
        })?;
    Ok(LogPump {
        handle,
        errors,
        stop,
    })
}
#[cfg(not(windows))]
fn atomic_replace(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)?;
    if let Some(parent) = target.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}
#[cfg(windows)]
fn atomic_replace(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH. Never unlink target first.
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0x1 | 0x8) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "vanta-scheduler-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn unavailable_gpu_waits_without_launching() {
        let dir = TestDir::new();
        let mut scheduler = Scheduler::new(dir.0.clone());
        scheduler
            .submit(
                "test".into(),
                "missing-executable".into(),
                vec![],
                Priority::Normal,
                1,
            )
            .unwrap();
        scheduler.tick(None);
        assert_eq!(scheduler.jobs[0].status, Status::Waiting);
        assert!(scheduler.jobs[0].pid.is_none());
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn corrupt_storage_is_preserved_and_blocks_submission() {
        let path = std::env::temp_dir().join(format!("vanta-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("scheduler.json"), b"{broken").unwrap();
        {
            let mut scheduler = Scheduler::new(path.clone());
            assert!(scheduler.last_error.is_some());
            assert!(
                scheduler
                    .submit("test".into(), "missing".into(), vec![], Priority::Low, 0)
                    .is_err()
            );
        }
        assert_eq!(
            std::fs::read(path.join("scheduler.json")).unwrap(),
            b"{broken"
        );
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(test)]
mod behavior_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    struct Dir(PathBuf);
    impl Dir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "vanta-behavior-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn add(s: &mut Scheduler, priority: Priority, mb: u64, fixture: &str) -> u64 {
        s.submit(
            "fixture".into(),
            std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            vec![
                "--exact".into(),
                format!("{}::{fixture}", module_path!().split_once("::").unwrap().1),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            priority,
            mb,
        )
        .unwrap()
    }
    fn finish(s: &mut Scheduler) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while s.jobs.iter().any(|j| j.status == Status::Running) {
            s.tick(None);
            assert!(
                Instant::now() < deadline,
                "child did not finish: {:?}",
                s.last_error
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    #[ignore]
    fn child_wait() {
        thread::sleep(Duration::from_secs(60));
    }
    #[test]
    #[ignore]
    fn child_ok() {
        println!("owned child success");
    }
    #[test]
    #[ignore]
    fn child_flood() {
        let bytes = vec![b'x'; MAX_LOG * 2];
        std::io::stdout().write_all(&bytes).unwrap();
        std::io::stderr().write_all(&bytes).unwrap();
    }
    #[test]
    #[ignore]
    fn child_self_recovery() {
        let directory = PathBuf::from(std::env::var_os("VANTA_SELF_RECOVERY_DIR").unwrap());
        let mode = std::env::var("VANTA_SELF_RECOVERY_MODE").unwrap();
        let mut scheduler = Scheduler::new(directory);
        if mode == "queue" {
            let fixture = format!("{}::child_ok", module_path!().split_once("::").unwrap().1);
            scheduler
                .submit_self(
                    "Built-in fixture".into(),
                    vec!["--exact".into(), fixture, "--ignored".into()],
                    Priority::Normal,
                    0,
                )
                .unwrap();
        } else {
            scheduler.allow_unmonitored = true;
            scheduler.set_paused(false);
            scheduler.tick(None);
            finish(&mut scheduler);
            assert_eq!(
                scheduler.jobs[0].status,
                Status::Completed,
                "{:?}",
                scheduler.last_error
            );
        }
    }
    #[test]
    fn built_in_workload_recovers_after_the_old_installation_directory_is_removed() {
        let dir = Dir::new();
        let old = dir.0.join("old.exe");
        let updated = dir.0.join("updated.exe");
        fs::copy(std::env::current_exe().unwrap(), &old).unwrap();
        fs::copy(&old, &updated).unwrap();
        let queue = dir.0.join("queue");
        let fixture = format!(
            "{}::child_self_recovery",
            module_path!().split_once("::").unwrap().1
        );
        let run = |executable: &Path, mode: &str| {
            let output = Command::new(executable)
                .args(["--exact", &fixture, "--ignored", "--nocapture"])
                .env("VANTA_SELF_RECOVERY_DIR", &queue)
                .env("VANTA_SELF_RECOVERY_MODE", mode)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&old, "queue");
        fs::remove_file(old).unwrap();
        run(&updated, "recover");
    }
    #[test]
    fn strict_priority_blocks_lower_job_that_fits() {
        let dir = Dir::new();
        let mut s = Scheduler::new(dir.0.clone());
        add(&mut s, Priority::Low, 1, "child_wait");
        let high = add(&mut s, Priority::Critical, 600, "child_wait");
        s.tick(Some(GpuBudget {
            total_mb: 2048,
            used_mb: 1024,
        }));
        assert_eq!(
            s.jobs.iter().find(|j| j.id == high).unwrap().status,
            Status::Waiting
        );
        assert_eq!(s.jobs[0].status, Status::Queued);
        s.tick(Some(GpuBudget {
            total_mb: 2048,
            used_mb: 936,
        }));
        assert_eq!(
            s.jobs.iter().find(|j| j.id == high).unwrap().status,
            Status::Running
        );
        s.tick(Some(GpuBudget {
            total_mb: 2048,
            used_mb: 0,
        }));
        assert_eq!(
            s.jobs
                .iter()
                .filter(|j| j.status == Status::Running)
                .count(),
            1
        );
        s.cancel(high).unwrap();
        assert_eq!(s.jobs[1].status, Status::Cancelled);
        assert!(s.jobs[1].pid.is_none());
    }
    #[test]
    fn impossible_request_rejected_before_next_candidate() {
        let dir = Dir::new();
        let mut s = Scheduler::new(dir.0.clone());
        add(&mut s, Priority::Critical, 513, "child_ok");
        add(&mut s, Priority::Low, 512, "child_ok");
        s.tick(Some(GpuBudget {
            total_mb: 1024,
            used_mb: 0,
        }));
        assert_eq!(s.jobs[0].status, Status::Failed);
        assert_eq!(s.jobs[1].status, Status::Running);
        finish(&mut s);
        assert_eq!(s.jobs[1].status, Status::Completed);
        assert_eq!(s.jobs[1].exit_code, Some(0));
    }
    #[test]
    fn pause_stops_dispatch_but_still_polls_and_shutdown_kills_owned() {
        let dir = Dir::new();
        let mut s = Scheduler::new(dir.0.clone());
        s.allow_unmonitored = true;
        let id = add(&mut s, Priority::Normal, 1, "child_wait");
        s.set_paused(true);
        s.tick(None);
        assert_eq!(s.jobs[0].status, Status::Queued);
        s.set_paused(false);
        s.tick(None);
        assert_eq!(s.jobs[0].status, Status::Running);
        s.shutdown();
        assert_eq!(s.jobs[0].status, Status::Cancelled);
        assert!(s.cancel(id).is_err());
    }
    #[test]
    fn recovery_interrupts_running_and_requires_explicit_resume() {
        let dir = Dir::new();
        {
            let mut s = Scheduler::new(dir.0.clone());
            add(&mut s, Priority::High, 1, "child_ok");
            add(&mut s, Priority::Low, 1, "child_ok");
        }
        // A crashed controller left a Running record with an arbitrary stale PID.
        let path = dir.0.join("scheduler.json");
        let mut document: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        document["jobs"][0]["status"] = "Running".into();
        document["jobs"][0]["pid"] = std::process::id().into();
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        let mut s = Scheduler::new(dir.0.clone());
        assert_eq!(s.jobs[0].status, Status::Interrupted);
        assert!(s.jobs[0].pid.is_none());
        s.allow_unmonitored = true;
        s.paused = false; // Public UI state cannot bypass recovery acknowledgement.
        s.tick(None);
        assert_eq!(s.jobs[1].status, Status::Queued);
        s.set_paused(false);
        s.tick(None);
        finish(&mut s);
        assert_eq!(s.jobs[1].status, Status::Completed);
    }
    #[test]
    fn log_streams_are_drained_and_bounded() {
        let dir = Dir::new();
        let mut s = Scheduler::new(dir.0.clone());
        let id = add(&mut s, Priority::Normal, 1, "child_flood");
        s.allow_unmonitored = true;
        s.tick(None);
        finish(&mut s);
        assert_eq!(s.jobs[0].status, Status::Completed);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            s.tick(None);
            let stdout = fs::metadata(dir.0.join("logs").join(format!("{id}-stdout.log")))
                .unwrap()
                .len();
            let stderr = fs::metadata(dir.0.join("logs").join(format!("{id}-stderr.log")))
                .unwrap()
                .len();
            if stdout == MAX_LOG as u64 && stderr == MAX_LOG as u64 {
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        assert!(s.last_error.is_none(), "{:?}", s.last_error);
    }
    #[test]
    fn durable_argv_is_literal_and_events_and_history_are_bounded() {
        let dir = Dir::new();
        let argv = vec![
            "two words".into(),
            "$(unexpected) & | ;".into(),
            "".into(),
            "quote\"literal".into(),
        ];
        {
            let mut s = Scheduler::new(dir.0.clone());
            s.submit(
                "argv".into(),
                "missing".into(),
                argv.clone(),
                Priority::Normal,
                1,
            )
            .unwrap();
            for _ in 0..140 {
                let id = s
                    .submit(
                        "history".into(),
                        "missing".into(),
                        vec![],
                        Priority::Normal,
                        1,
                    )
                    .unwrap();
                s.cancel(id).unwrap();
            }
            assert!(s.events.len() <= MAX_EVENTS);
            assert_eq!(s.jobs.len(), MAX_HISTORY + 1);
        }
        let s = Scheduler::new(dir.0.clone());
        assert_eq!(s.jobs[0].args, argv);
        assert!(s.paused);
        assert!(s.events.len() <= MAX_EVENTS);
    }
    #[test]
    #[ignore]
    fn child_argv() {
        println!(
            "ARGV:{}",
            serde_json::to_string(&std::env::args().collect::<Vec<_>>()).unwrap()
        );
    }
    #[test]
    fn arguments_reach_executable_without_shell_expansion() {
        let dir = Dir::new();
        let mut s = Scheduler::new(dir.0.clone());
        let mut args = vec![
            "--exact".into(),
            format!("{}::child_argv", module_path!().split_once("::").unwrap().1),
            "--ignored".into(),
            "--nocapture".into(),
        ];
        let payload = "two words & | ; $(no-expansion) quote\"literal";
        args.extend(["--skip".into(), payload.into()]);
        let id = s
            .submit(
                "argv".into(),
                std::env::current_exe()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                args,
                Priority::Normal,
                1,
            )
            .unwrap();
        s.allow_unmonitored = true;
        s.tick(None);
        finish(&mut s);
        assert_eq!(s.jobs[0].status, Status::Completed);
        let path = dir.0.join("logs").join(format!("{id}-stdout.log"));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let output = fs::read_to_string(&path).unwrap();
            if let Some(json) = output.lines().find_map(|line| line.strip_prefix("ARGV:")) {
                let observed: Vec<String> = serde_json::from_str(json).unwrap();
                assert_eq!(observed.last().unwrap(), payload);
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn equal_priority_is_fifo_and_spawn_failure_releases_slot() {
        let dir = Dir::new();
        let mut s = Scheduler::new(dir.0.clone());
        s.submit(
            "missing".into(),
            dir.0.join("missing.exe").to_string_lossy().into_owned(),
            vec![],
            Priority::High,
            0,
        )
        .unwrap();
        let first = add(&mut s, Priority::Normal, 0, "child_wait");
        let second = add(&mut s, Priority::Normal, 0, "child_wait");
        s.allow_unmonitored = true;
        s.tick(None);
        assert_eq!(s.jobs[0].status, Status::Failed);
        assert_eq!(
            s.jobs.iter().find(|j| j.id == first).unwrap().status,
            Status::Running
        );
        assert_eq!(
            s.jobs.iter().find(|j| j.id == second).unwrap().status,
            Status::Queued
        );
        assert!(s.last_error.as_ref().unwrap().contains("Spawn job"));
    }
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn child_heartbeat() {
        let path = PathBuf::from(std::env::args().next_back().unwrap());
        for sequence in 0..150 {
            fs::write(&path, sequence.to_string()).unwrap();
            thread::sleep(Duration::from_millis(20));
        }
    }
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn child_tree() {
        let path = std::env::args().next_back().unwrap();
        let fixture = format!(
            "{}::child_heartbeat",
            module_path!().split_once("::").unwrap().1
        );
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &fixture,
                "--ignored",
                "--nocapture",
                "--skip",
                &path,
            ])
            .spawn()
            .unwrap();
        let _ = child.wait();
    }
    #[cfg(windows)]
    #[test]
    fn cancellation_stops_owned_descendants() {
        let dir = Dir::new();
        let heartbeat = dir.0.join("heartbeat");
        let mut s = Scheduler::new(dir.0.clone());
        let fixture = format!("{}::child_tree", module_path!().split_once("::").unwrap().1);
        let id = s
            .submit(
                "tree".into(),
                std::env::current_exe()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                vec![
                    "--exact".into(),
                    fixture,
                    "--ignored".into(),
                    "--nocapture".into(),
                    "--skip".into(),
                    heartbeat.to_string_lossy().into_owned(),
                ],
                Priority::Normal,
                0,
            )
            .unwrap();
        s.allow_unmonitored = true;
        s.tick(None);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !heartbeat.exists() {
            assert!(
                Instant::now() < deadline,
                "fixture did not start: {:?}",
                s.last_error
            );
            thread::sleep(Duration::from_millis(10));
        }
        s.cancel(id).unwrap();
        thread::sleep(Duration::from_millis(100));
        let stopped = fs::read(&heartbeat).unwrap();
        thread::sleep(Duration::from_millis(200));
        assert_eq!(
            fs::read(&heartbeat).unwrap(),
            stopped,
            "descendant survived cancellation"
        );
    }
    #[cfg(windows)]
    #[test]
    #[ignore]
    // Intentionally exit before waiting: the controller must terminate the descendant.
    #[allow(clippy::zombie_processes)]
    fn child_tree_exit() {
        let path = std::env::args().next_back().unwrap();
        let fixture = format!(
            "{}::child_heartbeat",
            module_path!().split_once("::").unwrap().1
        );
        let _child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &fixture,
                "--ignored",
                "--nocapture",
                "--skip",
                &path,
            ])
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !Path::new(&path).exists() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
    }
    #[cfg(windows)]
    #[test]
    fn direct_exit_stops_descendants_and_next_job_can_run() {
        let dir = Dir::new();
        let heartbeat = dir.0.join("heartbeat");
        let mut s = Scheduler::new(dir.0.clone());
        let fixture = format!(
            "{}::child_tree_exit",
            module_path!().split_once("::").unwrap().1
        );
        s.submit(
            "tree".into(),
            std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            vec![
                "--exact".into(),
                fixture,
                "--ignored".into(),
                "--nocapture".into(),
                "--skip".into(),
                heartbeat.to_string_lossy().into_owned(),
            ],
            Priority::High,
            0,
        )
        .unwrap();
        add(&mut s, Priority::Normal, 0, "child_ok");
        s.allow_unmonitored = true;
        s.tick(None);
        let deadline = Instant::now() + Duration::from_secs(5);
        while s.jobs.iter().any(|j| j.status != Status::Completed) {
            s.tick(None);
            assert!(
                Instant::now() < deadline,
                "queue did not drain: {:?}",
                s.last_error
            );
            thread::sleep(Duration::from_millis(10));
        }
        thread::sleep(Duration::from_millis(100));
        let stopped = fs::read(&heartbeat).unwrap();
        thread::sleep(Duration::from_millis(100));
        assert_eq!(fs::read(&heartbeat).unwrap(), stopped);
    }
    #[cfg(windows)]
    #[test]
    fn stopped_log_reader_does_not_wait_for_inherited_writer() {
        // Exercise the private log seam with a writer deliberately outside containment.
        let dir = Dir::new();
        let fixture = format!(
            "{}::child_heartbeat",
            module_path!().split_once("::").unwrap().1
        );
        let ready = dir.0.join("writer-ready");
        let mut writer = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &fixture,
                "--ignored",
                "--nocapture",
                "--skip",
                &ready.to_string_lossy(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let reader = pump(
            writer.stdout.take().unwrap(),
            File::create(dir.0.join("log")).unwrap(),
        )
        .unwrap();
        // Wait until libtest has printed its banner and entered the silent fixture.
        let ready_deadline = Instant::now() + Duration::from_secs(2);
        while !ready.exists() && Instant::now() < ready_deadline {
            thread::sleep(Duration::from_millis(10));
        }
        reader
            .stop
            .store(true, std::sync::atomic::Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !reader.handle.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let finished = reader.handle.is_finished();
        let still_running = writer.try_wait().unwrap().is_none();
        if still_running {
            writer.kill().unwrap();
        }
        writer.wait().unwrap();
        reader.handle.join().unwrap();
        assert!(ready.exists(), "writer fixture did not become ready");
        assert!(
            still_running,
            "writer exited before log cancellation was checked"
        );
        assert!(
            finished,
            "log reader waited for an open inherited output handle"
        );
    }
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn child_controller() {
        let dir = PathBuf::from(std::env::args().next_back().unwrap());
        let mut s = Scheduler::new(dir.clone());
        let fixture = format!("{}::child_tree", module_path!().split_once("::").unwrap().1);
        s.submit(
            "tree".into(),
            std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            vec![
                "--exact".into(),
                fixture,
                "--ignored".into(),
                "--nocapture".into(),
                "--skip".into(),
                dir.join("heartbeat").to_string_lossy().into_owned(),
            ],
            Priority::Normal,
            0,
        )
        .unwrap();
        s.allow_unmonitored = true;
        for _ in 0..500 {
            s.tick(None);
            thread::sleep(Duration::from_millis(10));
        }
    }
    #[cfg(windows)]
    #[test]
    fn controller_crash_closes_job_and_recovery_never_relaunches() {
        let dir = Dir::new();
        let fixture = format!(
            "{}::child_controller",
            module_path!().split_once("::").unwrap().1
        );
        let mut controller = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &fixture,
                "--ignored",
                "--nocapture",
                "--skip",
                &dir.0.to_string_lossy(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let heartbeat = dir.0.join("heartbeat");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !heartbeat.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        // Kill only our live controller Child; this bypasses Scheduler::drop entirely.
        controller.kill().unwrap();
        controller.wait().unwrap();
        assert!(
            heartbeat.exists(),
            "controller fixture never launched its workload"
        );
        thread::sleep(Duration::from_millis(100));
        let stopped = fs::read(&heartbeat).unwrap();
        thread::sleep(Duration::from_millis(150));
        assert_eq!(
            fs::read(&heartbeat).unwrap(),
            stopped,
            "kill-on-close failed after controller crash"
        );
        let mut recovered = Scheduler::new(dir.0.clone());
        assert_eq!(recovered.jobs[0].status, Status::Interrupted);
        assert!(recovered.paused);
        assert!(recovered.jobs[0].pid.is_none());
        recovered.allow_unmonitored = true;
        recovered.tick(None);
        assert_eq!(recovered.jobs[0].status, Status::Interrupted);
    }
    #[test]
    fn running_observation_waits_two_seconds_then_emits_once_without_changing_reason() {
        let dir = Dir::new();
        let mut s = Scheduler::new(dir.0.clone());
        add(&mut s, Priority::Normal, 1, "child_wait");
        let baseline = GpuBudget {
            total_mb: 4096,
            used_mb: 1000,
        };
        s.tick(Some(baseline));
        let reason = s.jobs[0].reason.clone();
        let pid = s.jobs[0].pid.unwrap();
        s.tick(Some(GpuBudget {
            used_mb: 1200,
            ..baseline
        }));
        assert!(
            !s.events
                .iter()
                .any(|e| e.starts_with("Observed global VRAM delta"))
        );
        s.owned
            .as_mut()
            .unwrap()
            .observation
            .as_mut()
            .unwrap()
            .started = Instant::now() - Duration::from_secs(2);
        s.tick(None);
        s.tick(Some(GpuBudget {
            total_mb: 4096,
            used_mb: 5000,
        }));
        s.tick(Some(GpuBudget {
            total_mb: 8192,
            used_mb: 1200,
        }));
        assert!(
            !s.events
                .iter()
                .any(|e| e.starts_with("Observed global VRAM delta"))
        );
        s.set_paused(true); // Pausing admission must not pause running observations.
        s.tick(Some(GpuBudget {
            used_mb: 1200,
            ..baseline
        }));
        s.tick(Some(GpuBudget {
            used_mb: 1300,
            ..baseline
        }));
        let observations: Vec<_> = s
            .events
            .iter()
            .filter(|e| e.starts_with("Observed global VRAM delta"))
            .collect();
        assert_eq!(observations.len(), 1);
        assert_eq!(
            observations[0],
            &format!(
                "Observed global VRAM delta +200 MB while PID {pid} running; not attributable per-process allocation"
            )
        );
        assert_eq!(s.jobs[0].reason, reason);
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.0.join("scheduler.json")).unwrap()).unwrap();
        assert!(stored["events"].as_array().unwrap().iter().any(|e| {
            e.as_str()
                .is_some_and(|e| e.starts_with("Observed global VRAM delta"))
        }));
    }
    #[test]
    fn observation_reports_negative_global_delta() {
        let dir = Dir::new();
        let mut s = Scheduler::new(dir.0.clone());
        add(&mut s, Priority::Normal, 1, "child_wait");
        s.tick(Some(GpuBudget {
            total_mb: 4096,
            used_mb: 1000,
        }));
        s.owned
            .as_mut()
            .unwrap()
            .observation
            .as_mut()
            .unwrap()
            .started = Instant::now() - Duration::from_secs(2);
        s.tick(Some(GpuBudget {
            total_mb: 4096,
            used_mb: 700,
        }));
        assert!(
            s.events
                .iter()
                .any(|e| e.starts_with("Observed global VRAM delta -300 MB while PID "))
        );
    }
    #[test]
    fn exited_child_is_polled_before_overdue_observation() {
        let dir = Dir::new();
        let mut s = Scheduler::new(dir.0.clone());
        add(&mut s, Priority::Normal, 1, "child_wait");
        s.tick(Some(GpuBudget {
            total_mb: 4096,
            used_mb: 1000,
        }));
        let owned = s.owned.as_mut().unwrap();
        owned.child.kill().unwrap();
        owned.child.wait().unwrap();
        owned.observation.as_mut().unwrap().started = Instant::now() - Duration::from_secs(2);
        s.tick(Some(GpuBudget {
            total_mb: 4096,
            used_mb: 1200,
        }));
        assert_ne!(s.jobs[0].status, Status::Running);
        assert!(
            !s.events
                .iter()
                .any(|e| e.starts_with("Observed global VRAM delta"))
        );
    }
    #[test]
    fn unmonitored_launch_never_invents_a_baseline() {
        let dir = Dir::new();
        let mut s = Scheduler::new(dir.0.clone());
        add(&mut s, Priority::Normal, 1, "child_wait");
        s.allow_unmonitored = true;
        s.tick(None);
        assert!(s.owned.as_ref().unwrap().observation.is_none());
        s.tick(Some(GpuBudget {
            total_mb: 4096,
            used_mb: 1200,
        }));
        assert!(s.owned.as_ref().unwrap().observation.is_none());
        assert!(
            !s.events
                .iter()
                .any(|e| e.starts_with("Observed global VRAM delta"))
        );
    }
    #[test]
    fn storage_failure_replaces_earlier_spawn_error() {
        let dir = Dir::new();
        let mut s = Scheduler::new(dir.0.clone());
        s.submit(
            "missing".into(),
            dir.0.join("missing.exe").to_string_lossy().into_owned(),
            vec![],
            Priority::Normal,
            1,
        )
        .unwrap();
        s.allow_unmonitored = true;
        s.tick(None);
        assert!(s.last_error.as_ref().unwrap().contains("Spawn job"));
        fs::create_dir(dir.0.join("scheduler.json.tmp")).unwrap();
        assert!(
            s.submit("next".into(), "missing".into(), vec![], Priority::Normal, 1)
                .is_err()
        );
        assert!(
            s.last_error
                .as_ref()
                .unwrap()
                .contains("Open temporary queue"),
            "{:?}",
            s.last_error
        );
    }
    #[test]
    fn storage_failure_is_visible_and_blocks_later_dispatch() {
        let dir = Dir::new();
        let mut s = Scheduler::new(dir.0.clone());
        fs::create_dir(dir.0.join("scheduler.json.tmp")).unwrap();
        assert!(
            s.submit("x".into(), "missing".into(), vec![], Priority::Normal, 1)
                .is_err()
        );
        assert!(s.last_error.is_some());
        assert!(s.paused);
        s.set_paused(false);
        assert!(s.paused);
        assert!(s.jobs.is_empty());
    }
}

#[cfg(windows)]
struct WindowsJob(std::os::windows::io::OwnedHandle);
#[cfg(windows)]
impl WindowsJob {
    fn new() -> io::Result<Self> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::System::JobObjects::*;
        // Null security attributes make the anonymous handle non-inheritable.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // No BREAKAWAY_OK or SILENT_BREAKAWAY_OK: subsequent descendants inherit membership.
        let ok = unsafe {
            SetInformationJobObject(
                handle.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle))
    }
    fn assign(&self, child: &Child) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        if unsafe {
            windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(
                self.0.as_raw_handle(),
                child.as_raw_handle(),
            )
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    /// Only the actual primary thread can be resumed, and only inside our Job.
    fn resume(&self, child: &Child) -> io::Result<()> {
        use std::os::windows::{io::AsRawHandle, process::ChildExt};
        use windows_sys::Win32::System::{JobObjects::IsProcessInJob, Threading::ResumeThread};
        let mut member = 0;
        // SAFETY: both live owned handles remain valid; member is writable.
        if unsafe { IsProcessInJob(child.as_raw_handle(), self.0.as_raw_handle(), &mut member) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        if member == 0 {
            return Err(io::Error::other(
                "Refusing to resume a child outside the controlling Job Object",
            ));
        }
        // SAFETY: std::Child owns this exact process's primary thread handle.
        let previous = unsafe { ResumeThread(child.main_thread_handle().as_raw_handle()) };
        if previous == u32::MAX {
            return Err(io::Error::last_os_error());
        }
        if previous != 1 {
            return Err(io::Error::other(
                "Unexpected primary thread suspension count",
            ));
        }
        Ok(())
    }
    fn terminate(&self) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        if unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0.as_raw_handle(), 1)
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
#[cfg(windows)]
trait LogReader: Read + Send + std::os::windows::io::AsRawHandle + 'static {}
#[cfg(windows)]
impl<T: Read + Send + std::os::windows::io::AsRawHandle + 'static> LogReader for T {}
#[cfg(not(windows))]
trait LogReader: Read + Send + 'static {}
#[cfg(not(windows))]
impl<T: Read + Send + 'static> LogReader for T {}
#[cfg(windows)]
fn pipe_available(reader: &impl std::os::windows::io::AsRawHandle) -> io::Result<u32> {
    // The existing windows-sys feature set excludes Pipes; keep this one documented FFI local.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn PeekNamedPipe(
            pipe: *mut std::ffi::c_void,
            buffer: *mut std::ffi::c_void,
            size: u32,
            read: *mut u32,
            available: *mut u32,
            left: *mut u32,
        ) -> i32;
    }
    let mut available = 0;
    if unsafe {
        PeekNamedPipe(
            reader.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut available,
            std::ptr::null_mut(),
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(available)
    }
}

#[cfg(all(test, windows))]
mod suspended_tests {
    use super::*;
    use std::{
        os::windows::process::CommandExt,
        time::{Duration, Instant},
    };
    struct Fixture(Child);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    #[test]
    fn suspended_child_cannot_resume_until_it_belongs_to_the_controlling_job() {
        let job = WindowsJob::new().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .arg("--list")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.creation_flags(0x08000004); // CREATE_NO_WINDOW | CREATE_SUSPENDED
        let mut child = Fixture(command.spawn().unwrap());
        assert!(
            job.resume(&child.0).is_err(),
            "uncontained child was resumed"
        );
        assert!(child.0.try_wait().unwrap().is_none());
        job.assign(&child.0).unwrap();
        job.resume(&child.0).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "contained child did not resume");
            thread::sleep(Duration::from_millis(10));
        }
    }
}
