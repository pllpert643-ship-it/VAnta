//! Local administration with explicit captures and confirmed changes.
//! OS calls run on one worker; rendering only polls its bounded result channel.

use crate::{maintenance, platform, release, services, startup};
use eframe::egui::{self, Color32, RichText};
use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const ACCENT: Color32 = Color32::from_rgb(169, 141, 255);
const MUTED: Color32 = Color32::from_rgb(143, 154, 178);
const GREEN: Color32 = Color32::from_rgb(119, 224, 177);
const RED: Color32 = Color32::from_rgb(255, 143, 154);
const MAINTENANCE_CAPTURE_AGE: Duration = Duration::from_secs(60);

/// Independent live inventories; call `show` from the administration page and
/// `shutdown` on application exit. Construction never captures or changes OS state.
/// Only one capture or confirmed change can be outstanding, across all panes.
pub struct AdminWorkspace {
    sender: Option<mpsc::SyncSender<Job>>,
    receiver: mpsc::Receiver<Reply>,
    thread: Option<JoinHandle<()>>,
    busy: Option<String>,
    notice: Option<(String, bool)>,
    services: Vec<services::ServiceRow>,
    services_captured: Option<Instant>,
    startup: Vec<startup::StartupEntry>,
    startup_captured: Option<Instant>,
    memory: Option<MaintenanceCapture>,
    service_filter: String,
    startup_filter: String,
    selected_service: Option<String>,
    selected_startup: Option<usize>,
    confirmation: Option<Mutation>,
    focus_cancel: bool,
}

struct MaintenanceCapture {
    memory: platform::PerformanceMemory,
    elevated: bool,
    captured: Instant,
}

#[derive(Clone)]
enum Mutation {
    Service {
        name: String,
        display_name: String,
        action: services::ServiceAction,
    },
    Startup {
        entry: startup::StartupEntry,
        enabled: bool,
    },
    Maintenance(maintenance::Operation),
}

impl Mutation {
    fn label(&self) -> &'static str {
        match self {
            Self::Service {
                action: services::ServiceAction::Start,
                ..
            } => "Start service",
            Self::Service {
                action: services::ServiceAction::Stop,
                ..
            } => "Stop service",
            Self::Service {
                action: services::ServiceAction::Restart,
                ..
            } => "Restart service",
            Self::Startup { enabled: true, .. } => "Enable startup entry",
            Self::Startup { enabled: false, .. } => "Disable startup entry",
            Self::Maintenance(operation) => operation_label(*operation),
        }
    }
}

#[derive(Clone, Copy)]
enum Area {
    Services,
    Startup,
    Memory,
}
enum Job {
    CaptureServices,
    CaptureStartup,
    CaptureMemory,
    Mutate(Mutation),
}
enum Reply {
    Services(Result<Vec<services::ServiceRow>, String>),
    Startup(Result<Vec<startup::StartupEntry>, String>),
    Memory(Result<MaintenanceCapture, String>),
    Changed(Area, Result<String, String>),
}

impl AdminWorkspace {
    pub fn new() -> Self {
        Self::with_executor(execute)
    }

    // The executor is an internal seam for headless tests. Test adapters never
    // invoke the real delegates, including when exercising confirmation.
    fn with_executor(mut execute: impl FnMut(Job) -> Reply + Send + 'static) -> Self {
        let (sender, jobs) = mpsc::sync_channel(1);
        let (results, receiver) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("vanta-administration".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    // Nonblocking results also let shutdown join without a UI drain.
                    if results.try_send(execute(job)).is_err() {
                        break;
                    }
                }
            });
        let (thread, sender, notice) = match thread {
            Ok(handle) => (Some(handle), Some(sender), None),
            Err(error) => (
                None,
                None,
                Some((
                    format!("Could not start administration worker: {error}"),
                    true,
                )),
            ),
        };
        Self {
            sender,
            receiver,
            thread,
            notice,
            busy: None,
            services: Vec::new(),
            services_captured: None,
            startup: Vec::new(),
            startup_captured: None,
            memory: None,
            service_filter: String::new(),
            startup_filter: String::new(),
            selected_service: None,
            selected_startup: None,
            confirmation: None,
            focus_cancel: false,
        }
    }

    /// True while a job is outstanding, including a completed result awaiting
    /// receipt by `show`. Reading this state does not poll or consume results.
    pub fn is_busy(&self) -> bool {
        self.busy.is_some()
    }

    /// True while a change awaits confirmation. Keep this workspace visible so
    /// `show` can render the modal or cancel it when its parent UI is disabled.
    pub fn has_confirmation(&self) -> bool {
        self.confirmation.is_some()
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.receive();
        ui.label(
            RichText::new(
                if release::STORE { "Inspect services, startup entries and memory counters. Manage system settings using Windows tools." } else { "Capture current inventories, inspect a target, then confirm each change." },
            )
            .color(ACCENT),
        );
        ui.label(
            RichText::new("Independent live Windows inventories / not historical snapshot data / no automatic cleanup or elevation")
                .small()
                .color(MUTED),
        );
        if let Some((message, error)) = &self.notice {
            ui.colored_label(if *error { RED } else { GREEN }, message);
        }
        if let Some(label) = &self.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(label);
            });
            ui.label(
                RichText::new(
                    "One operation at a time. Service transitions can take up to 10 seconds.",
                )
                .small()
                .color(MUTED),
            );
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        if self.sender.is_none() {
            ui.colored_label(RED, "Administration worker is stopped.");
        }
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(Color32::from_rgb(17, 20, 30))
            .inner_margin(14.0)
            .corner_radius(9.0)
            .show(ui, |ui| {
                egui::CollapsingHeader::new("Windows services")
                    .id_salt("admin-services")
                    .default_open(true)
                    .show(ui, |ui| self.services_pane(ui));
            });
        ui.add_space(10.0);
        egui::Frame::new()
            .fill(Color32::from_rgb(17, 20, 30))
            .inner_margin(14.0)
            .corner_radius(9.0)
            .show(ui, |ui| {
                egui::CollapsingHeader::new("Startup entries")
                    .id_salt("admin-startup")
                    .default_open(true)
                    .show(ui, |ui| self.startup_pane(ui));
            });
        ui.add_space(10.0);
        egui::Frame::new()
            .fill(Color32::from_rgb(17, 20, 30))
            .inner_margin(14.0)
            .corner_radius(9.0)
            .show(ui, |ui| {
                egui::CollapsingHeader::new(if release::STORE {
                    "System memory counters"
                } else {
                    "System memory maintenance"
                })
                .id_salt("admin-memory")
                .default_open(true)
                .show(ui, |ui| self.memory_pane(ui));
            });
        // A Modal is a separate egui layer and does not inherit the parent Ui's
        // disabled state. Honor the host's historical-view gate explicitly.
        if ui.is_enabled() {
            self.show_confirmation(ui.ctx());
        } else {
            self.confirmation = None;
            self.focus_cancel = false;
        }
    }

    fn available(&self) -> bool {
        self.sender.is_some() && self.busy.is_none() && self.confirmation.is_none()
    }

    fn services_pane(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.available(),
                    egui::Button::new("Capture service inventory"),
                )
                .clicked()
            {
                self.submit(Job::CaptureServices, "Capturing service inventory");
            }
            inventory_age(ui, self.services_captured, self.services.len());
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.service_filter)
                .hint_text("Filter service name, display name or state"),
        );
        let filter = self.service_filter.to_lowercase();
        let rows: Vec<_> = self
            .services
            .iter()
            .filter(|row| {
                filter.is_empty()
                    || row.name.to_lowercase().contains(&filter)
                    || row.display_name.to_lowercase().contains(&filter)
                    || row.state.to_lowercase().contains(&filter)
            })
            .collect();
        egui::ScrollArea::both()
            .id_salt("admin-services-list")
            .max_height(220.0)
            .show_rows(ui, 28.0, rows.len(), |ui, range| {
                for index in range {
                    let row = rows[index];
                    let selected = self.selected_service.as_deref() == Some(row.name.as_str());
                    if ui
                        .selectable_label(
                            selected,
                            format!(
                                "{} / {} / {} / PID {}",
                                row.display_name, row.name, row.state, row.pid
                            ),
                        )
                        .clicked()
                    {
                        self.selected_service = Some(row.name.clone());
                    }
                }
            });
        if let Some(row) = self
            .selected_service
            .as_ref()
            .and_then(|name| self.services.iter().find(|row| &row.name == name))
            .cloned()
        {
            ui.label(format!(
                "{} / service name: {} / {}",
                row.display_name, row.name, row.state
            ));
            ui.add_enabled_ui(
                !release::STORE && self.available() && self.services_captured.is_some(),
                |ui| {
                    ui.horizontal(|ui| {
                        for (action, label) in [
                            (services::ServiceAction::Start, "Start..."),
                            (services::ServiceAction::Stop, "Stop..."),
                            (services::ServiceAction::Restart, "Restart..."),
                        ] {
                            if ui.button(label).clicked() {
                                self.request_confirmation(Mutation::Service {
                                    name: row.name.clone(),
                                    display_name: row.display_name.clone(),
                                    action,
                                });
                            }
                        }
                    });
                },
            );
        }
        ui.label(RichText::new(if release::STORE { "Service inventory is read only. Use Windows Services to manage services." } else { "Stopping or restarting a service can interrupt applications and dependent services. Access depends on Windows permissions." }).small().color(MUTED));
    }

    fn startup_pane(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.available(),
                    egui::Button::new("Capture startup inventory"),
                )
                .clicked()
            {
                self.submit(Job::CaptureStartup, "Capturing startup inventory");
            }
            inventory_age(ui, self.startup_captured, self.startup.len());
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.startup_filter)
                .hint_text("Filter startup name, scope or command"),
        );
        let filter = self.startup_filter.to_lowercase();
        let rows: Vec<_> = self
            .startup
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                filter.is_empty()
                    || row.name.to_lowercase().contains(&filter)
                    || row.scope.to_lowercase().contains(&filter)
                    || row.command.to_lowercase().contains(&filter)
            })
            .collect();
        egui::ScrollArea::both()
            .id_salt("admin-startup-list")
            .max_height(200.0)
            .show_rows(ui, 28.0, rows.len(), |ui, range| {
                for index in range {
                    let (entry_index, entry) = rows[index];
                    let enabled = match entry.enabled {
                        Some(true) => "enabled",
                        Some(false) => "disabled",
                        None => "unknown",
                    };
                    if ui
                        .selectable_label(
                            self.selected_startup == Some(entry_index),
                            format!(
                                "{} / {} / {}{}",
                                entry.name,
                                entry.scope,
                                enabled,
                                if entry.writable { "" } else { " / read only" }
                            ),
                        )
                        .clicked()
                    {
                        self.selected_startup = Some(entry_index);
                    }
                }
            });
        if let Some(entry) = self
            .selected_startup
            .and_then(|index| self.startup.get(index))
            .cloned()
        {
            ui.label(format!("{} / {}", entry.name, entry.scope));
            ui.label(RichText::new(&entry.command).monospace());
            if let Some(enabled) = entry.enabled {
                if ui
                    .add_enabled(
                        self.available()
                            && !release::STORE
                            && self.startup_captured.is_some()
                            && entry.writable,
                        egui::Button::new(if enabled {
                            "Disable entry..."
                        } else {
                            "Enable entry..."
                        }),
                    )
                    .clicked()
                {
                    self.request_confirmation(Mutation::Startup {
                        entry,
                        enabled: !enabled,
                    });
                }
            } else {
                ui.colored_label(MUTED, "Enabled state is unknown; changes are unavailable.");
            }
        }
        ui.label(RichText::new("Startup enabled state may be unknown. Use Windows Settings > Apps > Startup to manage startup applications.").small().color(MUTED));
    }

    fn memory_pane(&mut self, ui: &mut egui::Ui) {
        if ui
            .add_enabled(
                self.available(),
                egui::Button::new("Capture memory and elevation status"),
            )
            .clicked()
        {
            self.submit(
                Job::CaptureMemory,
                "Capturing memory counters and elevation status",
            );
        }
        if let Some(capture) = &self.memory {
            ui.label(format!(
                "Captured {}s ago / {}",
                capture.captured.elapsed().as_secs(),
                if capture.elevated {
                    "elevated"
                } else {
                    "not elevated"
                }
            ));
            egui::Grid::new("admin-memory-counters")
                .num_columns(2)
                .show(ui, |ui| {
                    for (label, value) in [
                        ("System cache", capture.memory.system_cache_bytes),
                        ("Commit", capture.memory.commit_bytes),
                        ("Commit limit", capture.memory.commit_limit_bytes),
                        ("Paged kernel pool", capture.memory.kernel_paged_bytes),
                        ("Nonpaged kernel pool", capture.memory.kernel_nonpaged_bytes),
                    ] {
                        ui.label(label);
                        ui.monospace(format!("{:.1} MiB", value as f64 / 1_048_576.0));
                        ui.end_row();
                    }
                });
            if !capture.elevated && !release::STORE {
                ui.colored_label(
                    MUTED,
                    "Elevated permissions are required. No automatic elevation is performed.",
                );
            }
            if capture.captured.elapsed() > MAINTENANCE_CAPTURE_AGE && !release::STORE {
                ui.colored_label(MUTED, "Capture again: maintenance requires counters and elevation status from the last 60 seconds.");
            }
        } else {
            ui.label(
                "Capture live memory counters and elevation status before choosing an operation.",
            );
        }
        if release::STORE {
            ui.label("Memory counters are observations. System memory maintenance is unavailable in this edition.");
            return;
        }
        let enabled = self.available() && self.maintenance_ready();
        ui.add_enabled_ui(enabled, |ui| {
            if ui.button("Trim system file cache...").clicked() { self.request_confirmation(Mutation::Maintenance(maintenance::Operation::TrimFileCache)); }
            egui::CollapsingHeader::new("Advanced standby operations").id_salt("admin-advanced").show(ui, |ui| {
                ui.colored_label(RED, "Uses undocumented Windows interfaces. Behavior can vary by Windows version and privileges; purging can increase page faults and reduce performance.");
                if ui.button("Purge standby list...").clicked() { self.request_confirmation(Mutation::Maintenance(maintenance::Operation::PurgeStandby)); }
                if ui.button("Purge priority-zero standby...").clicked() { self.request_confirmation(Mutation::Maintenance(maintenance::Operation::PurgePriorityZero)); }
            });
        });
        ui.label(RichText::new("Cache accounting overlaps other counters; do not add it to physical RAM use. No automatic clearing. A result cannot attribute a measured number of bytes freed to an operation; recapture to observe current counters.").small().color(MUTED));
    }

    fn maintenance_ready(&self) -> bool {
        self.memory.as_ref().is_some_and(|capture| {
            capture.elevated && capture.captured.elapsed() <= MAINTENANCE_CAPTURE_AGE
        })
    }

    fn can_mutate(&self, mutation: &Mutation) -> bool {
        if release::STORE {
            return false;
        }
        if self.sender.is_none() || self.busy.is_some() {
            return false;
        }
        match mutation {
            Mutation::Service { name, .. } => {
                self.services_captured.is_some()
                    && self.services.iter().any(|row| &row.name == name)
            }
            Mutation::Startup { entry, enabled } => {
                self.startup_captured.is_some()
                    && entry.writable
                    && entry.enabled.is_some_and(|current| current != *enabled)
                    && self.startup.iter().any(|current| {
                        current.name == entry.name
                            && current.scope == entry.scope
                            && current.command == entry.command
                            && current.enabled == entry.enabled
                            && current.writable
                    })
            }
            Mutation::Maintenance(_) => self.maintenance_ready(),
        }
    }

    fn request_confirmation(&mut self, mutation: Mutation) -> bool {
        if !self.available() || !self.can_mutate(&mutation) {
            return false;
        }
        self.confirmation = Some(mutation);
        self.focus_cancel = true;
        true
    }

    fn confirm_pending(&mut self) -> bool {
        let Some(mutation) = self.confirmation.take() else {
            return false;
        };
        if !self.can_mutate(&mutation) {
            self.notice = Some((
                "The capture or permissions are no longer current. Capture again before retrying."
                    .into(),
                true,
            ));
            return false;
        }
        let label = mutation.label();
        self.submit(Job::Mutate(mutation), label)
    }

    fn show_confirmation(&mut self, context: &egui::Context) {
        let Some(mutation) = self.confirmation.clone() else {
            return;
        };
        let mut cancel = false;
        let mut confirm = false;
        let modal = egui::Modal::new(egui::Id::new("admin-action-confirmation")).show(context, |ui| {
            ui.set_max_width(520.0);
            ui.heading(mutation.label());
            match &mutation {
                Mutation::Service { name, display_name, .. } => {
                    ui.strong(display_name); ui.monospace(format!("Service name: {name}"));
                    ui.label("This changes a live Windows service. Applications or dependent services may be interrupted. The transition can take up to 10 seconds.");
                },
                Mutation::Startup { entry, enabled } => {
                    ui.strong(&entry.name); ui.label(format!("Scope: {}", entry.scope));
                    ui.label("Captured command (revalidated by the backend):");
                    egui::ScrollArea::vertical().id_salt("admin-confirm-command").max_height(160.0).show(ui, |ui| { ui.label(RichText::new(&entry.command).monospace()); });
                    ui.label(if *enabled { "This entry will be enabled for future sign-ins." } else { "This entry will be disabled for future sign-ins." });
                },
                Mutation::Maintenance(operation) => {
                    ui.label("Target: live system-wide Windows memory state.");
                    ui.label("May discard useful cached pages and increase future disk I/O and page faults. No performance improvement or number of bytes freed is guaranteed.");
                    if matches!(operation, maintenance::Operation::PurgeStandby | maintenance::Operation::PurgePriorityZero) {
                        ui.colored_label(RED, "Advanced operation: uses an undocumented Windows interface and may fail or behave differently across Windows versions.");
                    }
                    ui.label("Elevation is checked again by the worker before execution. Capture again after the attempt.");
                },
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let response = ui.button("Cancel");
                if self.focus_cancel { response.request_focus(); }
                cancel = response.clicked();
                confirm = ui.add_enabled(self.can_mutate(&mutation), egui::Button::new(RichText::new("Confirm change").color(RED))).clicked();
            });
        });
        self.focus_cancel = false;
        if cancel
            || modal.should_close()
            || context.input(|input| input.key_pressed(egui::Key::Escape))
        {
            self.confirmation = None;
        } else if confirm {
            self.confirm_pending();
        }
    }

    fn submit(&mut self, job: Job, label: &str) -> bool {
        if !self.available() {
            return false;
        }
        let Some(sender) = &self.sender else {
            return false;
        };
        match sender.try_send(job) {
            Ok(()) => {
                self.busy = Some(label.into());
                true
            }
            Err(error) => {
                self.notice = Some((format!("Administration worker unavailable: {error}"), true));
                false
            }
        }
    }

    fn receive(&mut self) {
        let reply = match self.receiver.try_recv() {
            Ok(reply) => reply,
            Err(mpsc::TryRecvError::Disconnected) if self.sender.is_some() => {
                self.busy = None;
                self.sender = None;
                self.confirmation = None;
                self.notice = Some((
                    "Administration worker stopped unexpectedly. Reopen the application to retry."
                        .into(),
                    true,
                ));
                return;
            }
            Err(_) => return,
        };
        self.busy = None;
        match reply {
            Reply::Services(result) => {
                self.services.clear();
                self.services_captured = None;
                self.selected_service = None;
                match result {
                    Ok(rows) => {
                        self.services = rows;
                        self.services_captured = Some(Instant::now());
                        self.notice = Some(("Live service inventory captured.".into(), false));
                    }
                    Err(error) => self.notice = Some((error, true)),
                }
            }
            Reply::Startup(result) => {
                self.startup.clear();
                self.startup_captured = None;
                self.selected_startup = None;
                match result {
                    Ok(rows) => {
                        self.startup = rows;
                        self.startup_captured = Some(Instant::now());
                        self.notice = Some(("Live startup inventory captured.".into(), false));
                    }
                    Err(error) => self.notice = Some((error, true)),
                }
            }
            Reply::Memory(result) => {
                self.memory = None;
                match result {
                    Ok(capture) => {
                        self.memory = Some(capture);
                        self.notice = Some((
                            "Live memory counters and elevation status captured.".into(),
                            false,
                        ));
                    }
                    Err(error) => self.notice = Some((error, true)),
                }
            }
            Reply::Changed(area, result) => {
                match area {
                    Area::Services => {
                        self.services_captured = None;
                        self.selected_service = None;
                    }
                    Area::Startup => {
                        self.startup_captured = None;
                        self.selected_startup = None;
                    }
                    Area::Memory => self.memory = None,
                }
                self.notice = Some(match result {
                    Ok(message) => (
                        format!("{message} Capture again to inspect current state."),
                        false,
                    ),
                    Err(error) => (
                        format!(
                            "{error} Capture again before retrying; the state may have changed."
                        ),
                        true,
                    ),
                });
            }
        }
    }

    /// Close the command channel and join the worker, including any confirmed
    /// operation already in flight. No queued work or result drain is required.
    /// Service polling is bounded by its backend; OS calls must return to join.
    pub fn shutdown(&mut self) {
        self.confirmation = None;
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.busy = None;
    }
}

impl Default for AdminWorkspace {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for AdminWorkspace {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn execute(job: Job) -> Reply {
    if release::STORE
        && let Job::Mutate(ref mutation) = job
    {
        let area = match mutation {
            Mutation::Service { .. } => Area::Services,
            Mutation::Startup { .. } => Area::Startup,
            Mutation::Maintenance(_) => Area::Memory,
        };
        return Reply::Changed(
            area,
            release::require_system_mutations().map(|()| String::new()),
        );
    }
    match job {
        Job::CaptureServices => Reply::Services(services::list()),
        Job::CaptureStartup => Reply::Startup(startup::list()),
        Job::CaptureMemory => Reply::Memory((|| {
            let elevated = maintenance::is_elevated()?;
            let memory = platform::performance_memory()?;
            Ok(MaintenanceCapture {
                memory,
                elevated,
                captured: Instant::now(),
            })
        })()),
        Job::Mutate(Mutation::Service { name, action, .. }) => {
            Reply::Changed(Area::Services, services::control(&name, action))
        }
        Job::Mutate(Mutation::Startup { entry, enabled }) => {
            Reply::Changed(Area::Startup, startup::set_enabled(&entry, enabled))
        }
        Job::Mutate(Mutation::Maintenance(operation)) => Reply::Changed(
            Area::Memory,
            (|| {
                if !maintenance::is_elevated()? {
                    return Err("Elevated permissions are required for memory maintenance.".into());
                }
                maintenance::execute(operation)
            })(),
        ),
    }
}

fn inventory_age(ui: &mut egui::Ui, captured: Option<Instant>, count: usize) {
    ui.label(
        RichText::new(match captured {
            Some(at) => format!("{count} entries / captured {}s ago", at.elapsed().as_secs()),
            None => "Capture required before changes".into(),
        })
        .small()
        .color(MUTED),
    );
}

fn operation_label(operation: maintenance::Operation) -> &'static str {
    match operation {
        maintenance::Operation::TrimFileCache => "Trim system file cache",
        maintenance::Operation::PurgeStandby => "Purge system standby list",
        maintenance::Operation::PurgePriorityZero => "Purge priority-zero standby list",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn render(workspace: &mut AdminWorkspace) {
        let context = egui::Context::default();
        let _ = context.run(egui::RawInput::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| workspace.show(ui));
        });
    }

    #[test]
    fn captures_use_one_worker_and_reject_overlap_until_result_is_received() {
        let (entered, started) = mpsc::channel();
        let (release, proceed) = mpsc::channel();
        let mut workspace = AdminWorkspace::with_executor(move |_| {
            entered.send(thread::current().id()).unwrap();
            proceed.recv_timeout(Duration::from_secs(3)).unwrap();
            Reply::Services(Ok(vec![services::ServiceRow {
                name: "test-service".into(),
                display_name: "Test Service".into(),
                state: "running".into(),
                pid: 123,
            }]))
        });
        assert!(!workspace.is_busy());
        assert!(workspace.submit(Job::CaptureServices, "capture"));
        assert!(workspace.is_busy());
        assert_ne!(
            started.recv_timeout(Duration::from_secs(2)).unwrap(),
            thread::current().id()
        );
        assert!(!workspace.submit(Job::CaptureStartup, "overlapping capture"));
        release.send(()).unwrap();
        assert!(workspace.is_busy());
        let deadline = Instant::now() + Duration::from_secs(2);
        while workspace.busy.is_some() {
            workspace.receive();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!workspace.is_busy());
        assert_eq!(workspace.services[0].name, "test-service");
        assert!(workspace.services_captured.is_some());
        workspace.shutdown();
        workspace.shutdown();
        assert!(!workspace.submit(Job::CaptureServices, "after shutdown"));
    }
    #[cfg(not(feature = "store"))]
    #[test]
    fn service_confirmation_defaults_to_cancel_and_escape_also_cancels() {
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = Arc::clone(&calls);
        let mut workspace = AdminWorkspace::with_executor(move |job| {
            assert!(
                matches!(job, Job::CaptureServices),
                "a mutation was executed"
            );
            worker_calls.fetch_add(1, Ordering::SeqCst);
            Reply::Services(Ok(vec![services::ServiceRow {
                name: "demo".into(),
                display_name: "Demonstration".into(),
                state: "running".into(),
                pid: 1,
            }]))
        });
        workspace.submit(Job::CaptureServices, "capture");
        let deadline = Instant::now() + Duration::from_secs(2);
        while workspace.busy.is_some() {
            workspace.receive();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        let action = Mutation::Service {
            name: "demo".into(),
            display_name: "Demonstration".into(),
            action: services::ServiceAction::Stop,
        };
        assert!(workspace.request_confirmation(action.clone()));
        let context = egui::Context::default();
        let draw = |workspace: &mut AdminWorkspace, key: Option<egui::Key>| {
            let mut input = egui::RawInput::default();
            if let Some(key) = key {
                input.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let _ = context.run(input, |context| {
                egui::CentralPanel::default().show(context, |ui| workspace.show(ui));
            });
        };
        draw(&mut workspace, None);
        assert!(workspace.confirmation.is_some());
        draw(&mut workspace, Some(egui::Key::Enter));
        assert!(
            workspace.confirmation.is_none(),
            "Enter should activate the initially focused Cancel button"
        );
        assert!(workspace.request_confirmation(action));
        draw(&mut workspace, None);
        draw(&mut workspace, Some(egui::Key::Escape));
        assert!(workspace.confirmation.is_none());
        workspace.shutdown();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    #[cfg(not(feature = "store"))]
    fn finish_job(workspace: &mut AdminWorkspace) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while workspace.busy.is_some() {
            workspace.receive();
            assert!(
                Instant::now() < deadline,
                "administration job did not finish"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[cfg(not(feature = "store"))]
    fn startup_entry() -> startup::StartupEntry {
        startup::StartupEntry {
            scope: "HKCU Run (64-bit)".into(),
            name: "Demo startup".into(),
            command: "\"C:\\Program Files\\Demo\\demo.exe\" --background".into(),
            enabled: Some(true),
            writable: true,
        }
    }

    #[cfg(not(feature = "store"))]
    #[test]
    fn startup_confirmation_preserves_command_and_requires_recapture_after_failure() {
        let (sent, mutations) = mpsc::channel();
        let mut workspace = AdminWorkspace::with_executor(move |job| match job {
            Job::CaptureStartup => Reply::Startup(Ok(vec![startup_entry()])),
            Job::Mutate(Mutation::Startup { entry, enabled }) => {
                sent.send((entry.command, enabled)).unwrap();
                Reply::Changed(Area::Startup, Err("Command changed; recapture".into()))
            }
            _ => panic!("unexpected job"),
        });
        let mutation = |entry| Mutation::Startup {
            entry,
            enabled: false,
        };
        assert!(!workspace.request_confirmation(mutation(startup_entry())));
        workspace.submit(Job::CaptureStartup, "capture");
        finish_job(&mut workspace);
        let mut unsupported = startup_entry();
        unsupported.writable = false;
        assert!(!workspace.request_confirmation(mutation(unsupported)));
        let mut unknown = startup_entry();
        unknown.enabled = None;
        assert!(!workspace.request_confirmation(mutation(unknown)));
        let mut changed = startup_entry();
        changed.command.push_str(" changed");
        assert!(!workspace.request_confirmation(mutation(changed)));
        assert!(workspace.request_confirmation(mutation(startup_entry())));
        render(&mut workspace);
        assert!(
            mutations.try_recv().is_err(),
            "opening the modal ran the change"
        );
        assert!(!workspace.submit(Job::CaptureServices, "capture during confirmation"));
        assert!(workspace.confirm_pending());
        finish_job(&mut workspace);
        assert_eq!(
            mutations.recv_timeout(Duration::from_secs(1)).unwrap(),
            (startup_entry().command, false)
        );
        assert!(workspace.startup_captured.is_none());
        assert!(workspace.notice.as_ref().unwrap().1);
        assert!(!workspace.request_confirmation(mutation(startup_entry())));
    }

    #[cfg(not(feature = "store"))]
    fn memory_capture(elevated: bool) -> MaintenanceCapture {
        MaintenanceCapture {
            memory: platform::PerformanceMemory {
                commit_bytes: 100,
                commit_limit_bytes: 1000,
                system_cache_bytes: 200,
                kernel_paged_bytes: 10,
                kernel_nonpaged_bytes: 20,
                handles: 2,
                threads: 3,
            },
            elevated,
            captured: Instant::now(),
        }
    }

    #[cfg(not(feature = "store"))]
    #[test]
    fn memory_maintenance_requires_recent_elevated_capture_and_rechecks_on_confirmation() {
        let (sent, mutations) = mpsc::channel();
        let mut captures = 0;
        let mut workspace = AdminWorkspace::with_executor(move |job| match job {
            Job::CaptureMemory => {
                captures += 1;
                Reply::Memory(Ok(memory_capture(captures > 1)))
            }
            Job::Mutate(Mutation::Maintenance(operation)) => {
                sent.send(operation_label(operation)).unwrap();
                Reply::Changed(
                    Area::Memory,
                    Ok("Operation completed; no freed-byte claim".into()),
                )
            }
            _ => panic!("unexpected job"),
        });
        let mutation = || Mutation::Maintenance(maintenance::Operation::PurgeStandby);
        assert!(!workspace.request_confirmation(mutation()));
        workspace.submit(Job::CaptureMemory, "capture not elevated");
        finish_job(&mut workspace);
        assert!(!workspace.request_confirmation(mutation()));
        workspace.submit(Job::CaptureMemory, "capture elevated");
        finish_job(&mut workspace);
        assert!(workspace.request_confirmation(mutation()));
        render(&mut workspace);
        assert!(mutations.try_recv().is_err());
        workspace.memory.as_mut().unwrap().captured = Instant::now() - Duration::from_secs(61);
        assert!(
            !workspace.confirm_pending(),
            "expired capture authorized maintenance"
        );
        assert!(mutations.try_recv().is_err());
        workspace.submit(Job::CaptureMemory, "recapture elevated");
        finish_job(&mut workspace);
        assert!(workspace.request_confirmation(mutation()));
        assert!(workspace.confirm_pending());
        finish_job(&mut workspace);
        assert_eq!(
            mutations.recv_timeout(Duration::from_secs(1)).unwrap(),
            "Purge system standby list"
        );
        assert!(
            workspace.memory.is_none(),
            "maintenance must require a new capture"
        );
    }
    #[cfg(not(feature = "store"))]
    #[test]
    fn confirmed_service_uses_worker_once_and_invalidates_inventory() {
        let (sent, changed) = mpsc::channel();
        let mut workspace = AdminWorkspace::with_executor(move |job| match job {
            Job::CaptureServices => Reply::Services(Ok(vec![services::ServiceRow {
                name: "demo".into(),
                display_name: "Demo".into(),
                state: "running".into(),
                pid: 1,
            }])),
            Job::Mutate(Mutation::Service { name, action, .. }) => {
                assert!(matches!(action, services::ServiceAction::Restart));
                sent.send((name, thread::current().id())).unwrap();
                Reply::Changed(Area::Services, Ok("Test adapter restarted service".into()))
            }
            _ => panic!("unexpected job"),
        });
        workspace.submit(Job::CaptureServices, "capture");
        finish_job(&mut workspace);
        assert!(workspace.request_confirmation(Mutation::Service {
            name: "demo".into(),
            display_name: "Demo".into(),
            action: services::ServiceAction::Restart,
        }));
        assert!(changed.try_recv().is_err());
        assert!(workspace.confirm_pending());
        assert!(!workspace.confirm_pending(), "confirmation was reused");
        finish_job(&mut workspace);
        let (name, worker) = changed.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(name, "demo");
        assert_ne!(worker, thread::current().id());
        assert!(workspace.services_captured.is_none());
        assert!(!workspace.notice.as_ref().unwrap().1);
    }

    #[test]
    fn shutdown_joins_inflight_job_without_draining_its_result() {
        let (started, entered) = mpsc::channel();
        let (release, proceed) = mpsc::channel();
        let mut workspace = AdminWorkspace::with_executor(move |_| {
            started.send(()).unwrap();
            proceed.recv_timeout(Duration::from_secs(2)).unwrap();
            Reply::Services(Ok(Vec::new()))
        });
        workspace.submit(Job::CaptureServices, "capture");
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        release.send(()).unwrap();
        workspace.shutdown();
        assert!(workspace.thread.is_none());
        assert!(workspace.sender.is_none());
        assert!(workspace.busy.is_none());
    }
    #[cfg(not(feature = "store"))]
    #[test]
    fn disabling_parent_ui_cancels_confirmation_and_shutdown_never_dispatches_it() {
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = Arc::clone(&calls);
        let mut workspace = AdminWorkspace::with_executor(move |_| {
            worker_calls.fetch_add(1, Ordering::SeqCst);
            panic!("unconfirmed action must not reach executor");
        });
        workspace.services = vec![services::ServiceRow {
            name: "demo".into(),
            display_name: "Demo".into(),
            state: "running".into(),
            pid: 1,
        }];
        workspace.services_captured = Some(Instant::now());
        let mutation = || Mutation::Service {
            name: "demo".into(),
            display_name: "Demo".into(),
            action: services::ServiceAction::Stop,
        };
        assert!(!workspace.has_confirmation());
        assert!(workspace.request_confirmation(mutation()));
        assert!(workspace.has_confirmation());
        let context = egui::Context::default();
        let _ = context.run(egui::RawInput::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                ui.add_enabled_ui(false, |ui| workspace.show(ui));
            });
        });
        assert!(
            !workspace.has_confirmation(),
            "disabled parent left a separate modal active"
        );
        assert!(workspace.request_confirmation(mutation()));
        workspace.shutdown();
        assert!(!workspace.has_confirmation());
        assert!(!workspace.confirm_pending());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn opening_and_rendering_administration_never_captures_or_mutates_automatically() {
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = Arc::clone(&calls);
        let mut workspace = AdminWorkspace::with_executor(move |_| {
            worker_calls.fetch_add(1, Ordering::SeqCst);
            Reply::Services(Ok(Vec::new()))
        });
        render(&mut workspace);
        render(&mut workspace);
        workspace.shutdown();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
