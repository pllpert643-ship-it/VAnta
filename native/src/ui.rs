use crate::{
    platform::{self, PriorityClass, ProcessIdentity},
    policy,
    scheduler::{GpuBudget, Priority, Scheduler, Status},
    snapshots, storage,
    telemetry::{ProcessRow, Snapshot, TelemetryWorker},
};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};
const BG: Color32 = Color32::from_rgb(9, 11, 17);
const PANEL: Color32 = Color32::from_rgb(17, 20, 30);
const ACCENT: Color32 = Color32::from_rgb(169, 141, 255);
const MUTED: Color32 = Color32::from_rgb(143, 154, 178);
const GREEN: Color32 = Color32::from_rgb(119, 224, 177);
const RED: Color32 = Color32::from_rgb(255, 143, 154);
const CYAN: Color32 = Color32::from_rgb(108, 210, 232);
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    Overview,
    Workloads,
    Processes,
    Memory,
    Policy,
    Administration,
    Settings,
}
impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Overview => "Resource overview",
            Self::Workloads => "GPU workloads",
            Self::Processes => "Task manager",
            Self::Memory => "Memory map",
            Self::Policy => "Policy lab",
            Self::Administration => "Administration",
            Self::Settings => "Settings",
        }
    }
    fn subtitle(self) -> &'static str {
        match self {
            Self::Overview => {
                "Observe your machine. Understand the pressure. Choose the next action."
            }
            Self::Workloads => {
                "Explainable admission, strict priority and one controlled job at a time."
            }
            Self::Processes => "Live observations. Select a row to inspect or manage a process.",
            Self::Memory => {
                "Physical RAM and virtual address mappings, with the accounting kept explicit."
            }
            Self::Policy => {
                "A deterministic simulation. These workloads never run on your machine."
            }
            Self::Administration => {
                if crate::release::STORE {
                    "Read-only service, startup and system memory inventories."
                } else {
                    "Services, startup applications and confirmed memory maintenance."
                }
            }
            Self::Settings => {
                "Choose how much work the observer does, and how the interface moves."
            }
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    reduced_motion: bool,
    interval_seconds: u64,
    history_seconds: u64,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            reduced_motion: system_reduced_motion(),
            interval_seconds: 2,
            history_seconds: 120,
        }
    }
}
#[derive(Clone)]
enum Action {
    Inspect(ProcessIdentity),
    Trim(ProcessIdentity),
    Terminate(ProcessIdentity),
    Priority(ProcessIdentity, PriorityClass),
    Affinity(ProcessIdentity, usize),
}
impl Action {
    fn label(&self) -> &'static str {
        match self {
            Self::Inspect(_) => "Inspect mappings",
            Self::Trim(_) => "Trim working set",
            Self::Terminate(_) => "End selected process",
            Self::Priority(_, _) => "Change process priority",
            Self::Affinity(_, _) => "Change processor affinity",
        }
    }
    fn target(&self) -> ProcessIdentity {
        match self {
            Self::Inspect(p)
            | Self::Trim(p)
            | Self::Terminate(p)
            | Self::Priority(p, _)
            | Self::Affinity(p, _) => *p,
        }
    }
}
enum OperationResult {
    Mappings(ProcessIdentity, Result<Vec<platform::MemoryRegion>, String>),
    Action(Result<String, String>),
}
struct PendingOperation {
    receiver: mpsc::Receiver<OperationResult>,
    handle: Option<std::thread::JoinHandle<()>>,
}
#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Memory,
    Cpu,
    Pid,
    Name,
    Read,
    Write,
}
pub struct VantaApp {
    logo: egui::TextureHandle,
    worker: TelemetryWorker,
    live: Option<Snapshot>,
    historical: Option<Snapshot>,
    scheduler: Scheduler,
    admin: crate::admin_ui::AdminWorkspace,
    page: Page,
    displayed_page: Page,
    transition_at: Instant,
    preferences: Preferences,
    data_dir: PathBuf,
    history: VecDeque<(u64, f32, f32, Option<f32>)>,
    search: String,
    sort: Sort,
    descending: bool,
    selected: Option<ProcessIdentity>,
    affinity_mask: String,
    paused: bool,
    notice: Option<(String, bool)>,
    confirmation: Option<Action>,
    confirmation_needs_focus: bool,
    operation: Option<PendingOperation>,
    mappings: Vec<platform::MemoryRegion>,
    mappings_for: Option<ProcessIdentity>,
    executable: String,
    arguments: String,
    job_name: String,
    requested_mb: u64,
    priority: Priority,
    pulse_mb: u64,
    pulse_seconds: u64,
    smoke: bool,
    started: Instant,
    last_tick: Instant,
    policy_capacity: u64,
    policy_slots: usize,
    policy_jobs: Vec<policy::Workload>,
    live_memory: Option<platform::PerformanceMemory>,
}
pub fn launch(data_dir: PathBuf, smoke: bool) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("VAnta | Resource observatory")
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/vanta-icon-256.png"))
                    .expect("embedded Vanta icon must be a valid PNG"),
            )
            .with_inner_size([1200.0, 760.0])
            .with_min_inner_size([960.0, 640.0]),
        renderer: eframe::Renderer::Glow,
        vsync: true,
        ..Default::default()
    };
    eframe::run_native(
        "VAnta Native",
        options,
        Box::new(move |cc| Ok(Box::new(VantaApp::new(cc, data_dir, smoke)))),
    )
}
impl VantaApp {
    fn new(cc: &eframe::CreationContext<'_>, data_dir: PathBuf, smoke: bool) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = BG;
        style.visuals.window_fill = PANEL;
        style.visuals.override_text_color = Some(Color32::from_rgb(224, 229, 241));
        style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(28, 32, 46);
        style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(45, 43, 68);
        style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(65, 52, 98);
        style.visuals.extreme_bg_color = Color32::from_rgb(12, 14, 22);
        style.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.35);
        style.visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(28, 32, 46);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(45, 43, 68);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(65, 52, 98);
        style.spacing.item_spacing = Vec2::new(12.0, 10.0);
        style.spacing.button_padding = Vec2::new(13.0, 9.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(27.0));
        cc.egui_ctx.set_style(style);
        if let Ok(font) = std::fs::read("C:/Windows/Fonts/tahoma.ttf") {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("windows-ui".into(), egui::FontData::from_owned(font).into());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "windows-ui".into());
            cc.egui_ctx.set_fonts(fonts);
        }
        let (preferences, notice) = match std::fs::read(data_dir.join("preferences.json")) {
            Ok(bytes) => match serde_json::from_slice::<Preferences>(&bytes) {
                Ok(p) => (p, None),
                Err(e) => (
                    Preferences::default(),
                    Some((format!("Settings could not be read: {e}"), true)),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Preferences::default(), None),
            Err(e) => (
                Preferences::default(),
                Some((format!("Settings could not be read: {e}"), true)),
            ),
        };
        let worker = TelemetryWorker::spawn();
        worker.set_interval(Duration::from_secs(
            preferences.interval_seconds.clamp(1, 10),
        ));
        let logo_data =
            eframe::icon_data::from_png_bytes(include_bytes!("../assets/vanta-logo-512.png"))
                .expect("embedded Vanta logo must be a valid PNG");
        let logo = cc.egui_ctx.load_texture(
            "vanta-logo",
            egui::ColorImage::from_rgba_unmultiplied(
                [logo_data.width as usize, logo_data.height as usize],
                &logo_data.rgba,
            ),
            egui::TextureOptions::LINEAR,
        );
        Self {
            logo,
            worker,
            live: None,
            historical: None,
            scheduler: Scheduler::new(data_dir.clone()),
            admin: crate::admin_ui::AdminWorkspace::new(),
            page: Page::Overview,
            displayed_page: Page::Overview,
            transition_at: Instant::now(),
            preferences,
            data_dir,
            history: VecDeque::new(),
            search: String::new(),
            sort: Sort::Memory,
            descending: true,
            selected: None,
            affinity_mask: "1".into(),
            paused: false,
            notice,
            confirmation: None,
            confirmation_needs_focus: false,
            operation: None,
            mappings: Vec::new(),
            mappings_for: None,
            executable: String::new(),
            arguments: "[]".into(),
            job_name: "Custom workload".into(),
            requested_mb: 256,
            priority: Priority::Normal,
            pulse_mb: 256,
            pulse_seconds: 20,
            smoke,
            started: Instant::now(),
            last_tick: Instant::now(),
            policy_capacity: 1536,
            policy_slots: 2,
            policy_jobs: policy::example(),
            live_memory: None,
        }
    }
    fn snapshot(&self) -> Option<&Snapshot> {
        self.historical.as_ref().or(self.live.as_ref())
    }
    fn receive(&mut self) {
        if let Some(s) = self.worker.latest() {
            self.history.push_back((
                s.unix_seconds,
                s.cpu_percent,
                percent(s.used_memory, s.total_memory),
                s.gpu
                    .as_ref()
                    .and_then(|g| g.utilization_percent.map(|x| x as f32)),
            ));
            let oldest = s
                .unix_seconds
                .saturating_sub(self.preferences.history_seconds.clamp(30, 600));
            while self.history.front().is_some_and(|x| x.0 < oldest) || self.history.len() > 600 {
                self.history.pop_front();
            }
            self.live = Some(s);
            self.live_memory = platform::performance_memory().ok();
        }
        if self.last_tick.elapsed() >= Duration::from_millis(250) {
            let budget = self
                .live
                .as_ref()
                .filter(|s| epoch().saturating_sub(s.unix_seconds) <= 10)
                .and_then(|s| s.gpu.as_ref())
                .map(|g| GpuBudget {
                    total_mb: g.total_bytes / 1048576,
                    used_mb: g.used_bytes / 1048576,
                });
            self.scheduler.tick(budget);
            self.last_tick = Instant::now();
        }
        let result = self
            .operation
            .as_ref()
            .and_then(|op| match op.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(OperationResult::Action(Err(
                    "The process worker stopped before returning a result.".into(),
                ))),
            });
        if let Some(result) = result {
            if let Some(mut op) = self.operation.take()
                && let Some(handle) = op.handle.take()
            {
                let _ = handle.join();
            }
            match result {
                OperationResult::Mappings(target, result) => match result {
                    Ok(regions) => {
                        self.mappings = regions;
                        self.mappings_for = Some(target);
                        self.notice = Some((
                            "Mappings captured. Virtual regions are not resident RAM.".into(),
                            false,
                        ));
                    }
                    Err(e) => {
                        self.mappings.clear();
                        self.mappings_for = None;
                        self.notice = Some((e, true));
                    }
                },
                OperationResult::Action(result) => {
                    self.mappings.clear();
                    self.mappings_for = None;
                    self.notice = Some(match result {
                        Ok(message) => (message, false),
                        Err(message) => (message, true),
                    });
                }
            }
        }
    }
    fn start_action(&mut self, action: Action) {
        if self.historical.is_some()
            || self.operation.is_some()
            || self.admin.is_busy()
            || self.admin.has_confirmation()
            || self.paused
            || !self
                .live
                .as_ref()
                .is_some_and(|s| epoch().saturating_sub(s.unix_seconds) <= 10)
        {
            self.notice = Some(("Action cancelled: a current live capture and an idle operation worker are required.".into(), true));
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let result = match action {
                Action::Inspect(t) => OperationResult::Mappings(t, platform::memory_regions(t)),
                Action::Trim(t) => OperationResult::Action(platform::trim_working_set(t)),
                Action::Terminate(t) => OperationResult::Action(platform::terminate_process(t)),
                Action::Priority(t, p) => OperationResult::Action(platform::set_priority(t, p)),
                Action::Affinity(t, mask) => {
                    OperationResult::Action(platform::set_affinity(t, mask))
                }
            };
            let _ = sender.send(result);
        });
        self.operation = Some(PendingOperation {
            receiver,
            handle: Some(handle),
        });
    }
    fn export_snapshot(&mut self) {
        let Some(snapshot) = self.snapshot().cloned() else {
            return;
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Native snapshot", &["json"])
            .set_file_name("vanta-snapshot.json")
            .save_file()
        {
            self.notice = Some(match snapshots::save(&path, &snapshot) {
                Ok(()) => (
                    "Snapshot saved. Process names and paths are included.".into(),
                    false,
                ),
                Err(e) => (e, true),
            });
        }
    }
    fn import_snapshot(&mut self) {
        if self.operation.is_some() || self.admin.is_busy() || self.admin.has_confirmation() {
            self.notice = Some((
                "Wait for the current process operation before opening a historical snapshot."
                    .into(),
                true,
            ));
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("VAnta Native snapshot", &["json"])
            .pick_file()
        {
            match snapshots::load(&path) {
                Ok(snapshot) => {
                    self.historical = Some(snapshot);
                    self.selected = None;
                    self.mappings.clear();
                    self.mappings_for = None;
                    self.page = Page::Memory;
                    self.notice = Some((
                        "Historical snapshot loaded. Process actions are disabled.".into(),
                        false,
                    ));
                }
                Err(e) => self.notice = Some((e, true)),
            }
        }
    }
    fn export_csv(&mut self) {
        let Some(snapshot) = self.snapshot() else {
            return;
        };
        let mut csv = String::from(
            "timestamp,pid,name,cpu_percent_total_machine,working_set_bytes,read_bytes_interval,written_bytes_interval\r\n",
        );
        for p in &snapshot.processes {
            csv.push_str(&format!(
                "{},{},{},{:.2},{},{},{}\r\n",
                snapshot.unix_seconds,
                p.pid,
                csv_cell(&p.name),
                p.cpu_percent,
                p.memory_bytes,
                p.read_bytes,
                p.written_bytes
            ));
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name("vanta-processes.csv")
            .save_file()
        {
            self.notice = Some(match storage::atomic_write(&path, csv.as_bytes()) {
                Ok(()) => ("Process observations exported to CSV.".into(), false),
                Err(e) => (e, true),
            });
        }
    }
    fn overview(&mut self, ui: &mut egui::Ui) {
        let Some(s) = self.snapshot().cloned() else {
            ui.spinner();
            ui.label("Waiting for the first observation...");
            return;
        };
        let gpu_pct = s.gpu.as_ref().and_then(|g| g.utilization_percent);
        let ram_pct = percent(s.used_memory, s.total_memory);
        ui.columns(4, |c| {
            metric(
                &mut c[0],
                "CPU",
                &format!("{:.1}%", s.cpu_percent),
                &format!("{} logical processors", s.cpu_count),
                s.cpu_percent,
                CYAN,
                self.preferences.reduced_motion,
            );
            metric(
                &mut c[1],
                "PHYSICAL RAM",
                &format!("{:.1}%", ram_pct),
                &format!("{} / {}", bytes(s.used_memory), bytes(s.total_memory)),
                ram_pct,
                ACCENT,
                self.preferences.reduced_motion,
            );
            metric(
                &mut c[2],
                "GPU",
                &gpu_pct
                    .map(|p| format!("{p}%"))
                    .unwrap_or("Unavailable".into()),
                s.gpu
                    .as_ref()
                    .map(|g| g.name.as_str())
                    .unwrap_or("NVML not available"),
                gpu_pct.unwrap_or(0) as f32,
                GREEN,
                self.preferences.reduced_motion,
            );
            metric(
                &mut c[3],
                "VRAM",
                &s.gpu
                    .as_ref()
                    .map(|g| bytes(g.used_bytes))
                    .unwrap_or("Unavailable".into()),
                &s.gpu
                    .as_ref()
                    .map(|g| format!("{} capacity", bytes(g.total_bytes)))
                    .unwrap_or("No synthetic values".into()),
                s.gpu
                    .as_ref()
                    .map(|g| percent(g.used_bytes, g.total_bytes))
                    .unwrap_or(0.0),
                Color32::from_rgb(239, 185, 117),
                self.preferences.reduced_motion,
            );
        });
        ui.add_space(14.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Resource timeline");
                ui.label(
                    RichText::new(format!(
                        "{}s window / real observations",
                        self.preferences.history_seconds
                    ))
                    .color(MUTED),
                );
            });
            if self.historical.is_some() {
                ui.label("Live timeline hidden while reviewing a historical snapshot.");
            } else {
                timeline(ui, &self.history, self.preferences.history_seconds);
            }
            ui.horizontal(|ui| {
                ui.colored_label(CYAN, "CPU");
                ui.colored_label(ACCENT, "RAM");
                ui.colored_label(GREEN, "GPU (when available)");
            });
        });
        ui.add_space(10.0);
        ui.columns(2,|c|{card(&mut c[0],|ui|{ui.strong("What the observations say");let age=epoch().saturating_sub(s.unix_seconds);
if age>10{ui.colored_label(RED,format!("Observation is {age}s old. Admission requires fresh GPU data."));}
if ram_pct>=90.0{ui.colored_label(RED,"RAM usage exceeds 90%. Inspect large processes before another workload.");}else{ui.label(format!("{} RAM is available to Windows.",bytes(s.available_memory)));}
if let Some(g)=&s.gpu{ui.label(format!("GPU headroom after 512 MiB reserve: {}",bytes(g.total_bytes.saturating_sub(g.used_bytes).saturating_sub(512*1048576))));
if let Some(t)=g.temperature_c{ui.label(format!("GPU temperature: {t} C. Limits depend on device model."));}
if let Some(p)=g.power_w{ui.label(format!("GPU board power: {p:.1} W"));}}else if let Some(e)=&s.gpu_error{ui.colored_label(MUTED,format!("GPU unavailable: {e}"));}ui.label(RichText::new("Measurements are not a hardware fault diagnosis. Cached memory is normally reusable.").small().color(MUTED));});card(&mut c[1],|ui|{ui.strong("Largest working sets");let mut processes:Vec<_>=s.processes.iter().collect();processes.sort_by_key(|p|std::cmp::Reverse(p.memory_bytes));for p in processes.iter().take(5){ui.horizontal(|ui|{ui.label(&p.name);ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{ui.monospace(bytes(p.memory_bytes));});});}ui.label(RichText::new("Working sets can share pages. Adding rows can double count RAM.").small().color(MUTED));});});
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} processes / capture {} ms / {}s ago",
                    s.processes.len(),
                    s.collection_ms,
                    epoch().saturating_sub(s.unix_seconds)
                ))
                .small()
                .color(MUTED),
            );
            if ui.button("Export snapshot").clicked() {
                self.export_snapshot();
            }
            if ui.button("Inspect processes").clicked() {
                self.page = Page::Processes;
            }
        });
    }
    fn processes(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Search name, PID or executable path")
                    .desired_width(330.0)
                    .id_salt("process-search"),
            );
            ui.label("Sort");
            egui::ComboBox::from_id_salt("process-sort")
                .selected_text(match self.sort {
                    Sort::Memory => "Working set",
                    Sort::Cpu => "CPU",
                    Sort::Pid => "PID",
                    Sort::Name => "Name",
                    Sort::Read => "Read bytes",
                    Sort::Write => "Written bytes",
                })
                .show_ui(ui, |ui| {
                    for (value, label) in [
                        (Sort::Memory, "Working set"),
                        (Sort::Cpu, "CPU"),
                        (Sort::Pid, "PID"),
                        (Sort::Name, "Name"),
                        (Sort::Read, "Read bytes"),
                        (Sort::Write, "Written bytes"),
                    ] {
                        ui.selectable_value(&mut self.sort, value, label);
                    }
                });
            ui.checkbox(&mut self.descending, "Descending");
            if ui.button("Export CSV").clicked() {
                self.export_csv();
            }
        });
        let filter = self.search.to_lowercase();
        let mut rows: Vec<ProcessRow> = self
            .snapshot()
            .map(|s| {
                s.processes
                    .iter()
                    .filter(|p| {
                        filter.is_empty()
                            || p.name.to_lowercase().contains(&filter)
                            || p.pid.to_string().contains(&filter)
                            || p.executable
                                .as_ref()
                                .is_some_and(|s| s.to_lowercase().contains(&filter))
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        rows.sort_by(|a, b| {
            let order = match self.sort {
                Sort::Memory => a.memory_bytes.cmp(&b.memory_bytes),
                Sort::Cpu => a.cpu_percent.total_cmp(&b.cpu_percent),
                Sort::Pid => a.pid.cmp(&b.pid),
                Sort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                Sort::Read => a.read_bytes.cmp(&b.read_bytes),
                Sort::Write => a.written_bytes.cmp(&b.written_bytes),
            };
            let order = if self.descending {
                order.reverse()
            } else {
                order
            };
            order.then(a.pid.cmp(&b.pid))
        });
        ui.label(
            RichText::new(format!(
                "{} results / CPU normalized to total machine / I/O bytes since previous capture (includes non-disk I/O)",
                rows.len()
            ))
            .small()
            .color(MUTED),
        );
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("process-list")
            .max_height((ui.available_height() - 175.0).max(180.0))
            .show_rows(ui, 29.0, rows.len(), |ui, range| {
                for i in range {
                    let p = &rows[i];
                    let selected = self
                        .selected
                        .is_some_and(|x| x.pid == p.pid && x.start_time == p.start_time);
                    let text = format!(
                        "{:>7}   {:<30} CPU {:>5.1}% RAM {:>11} R {:>10} W {:>10}",
                        p.pid,
                        truncate(&p.name, 28),
                        p.cpu_percent,
                        bytes(p.memory_bytes),
                        bytes(p.read_bytes),
                        bytes(p.written_bytes)
                    );
                    if ui
                        .add_sized(
                            [ui.available_width(), 28.0],
                            egui::Button::new(RichText::new(text).monospace()).selected(selected),
                        )
                        .on_hover_text(
                            p.executable
                                .as_deref()
                                .unwrap_or("Executable path unavailable"),
                        )
                        .clicked()
                    {
                        let target = ProcessIdentity {
                            pid: p.pid,
                            start_time: p.start_time,
                            creation_filetime: None,
                        };
                        if self.historical.is_some() {
                            self.selected = Some(target);
                        } else {
                            match platform::pin_identity(target) {
                                Ok(target) => {
                                    self.selected = Some(target);
                                    if let Ok((mask, _)) = platform::affinity(target) {
                                        self.affinity_mask = format!("{mask:X}");
                                    }
                                }
                                Err(error) => {
                                    self.selected = Some(target);
                                    self.notice = Some((
                                        format!(
                                            "This process cannot be pinned for changes: {error}"
                                        ),
                                        true,
                                    ));
                                }
                            }
                        }
                    }
                }
            });
        ui.separator();
        if let Some(target) = self.selected {
            let current = self.snapshot().and_then(|s| {
                s.processes
                    .iter()
                    .find(|p| p.pid == target.pid && p.start_time == target.start_time)
            });
            ui.label(
                current
                    .map(|p| {
                        format!(
                            "Selected: {} / PID {} / started {}",
                            p.name, p.pid, p.start_time
                        )
                    })
                    .unwrap_or("Selected process is no longer present".into()),
            );
            let enabled = current.is_some()
                && self.historical.is_none()
                && !self.paused
                && self.operation.is_none()
                && !self.admin.is_busy()
                && !self.admin.has_confirmation()
                && target.creation_filetime.is_some()
                && self
                    .live
                    .as_ref()
                    .is_some_and(|s| epoch().saturating_sub(s.unix_seconds) <= 10);
            ui.add_enabled_ui(enabled, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Inspect memory regions").clicked() {
                        self.start_action(Action::Inspect(target));
                        self.page = Page::Memory;
                    }
                    if ui.button("Trim working set...").clicked() {
                        self.confirmation = Some(Action::Trim(target));
                        self.confirmation_needs_focus = true;
                    }
                    if ui
                        .button(RichText::new("End process...").color(RED))
                        .clicked()
                    {
                        self.confirmation = Some(Action::Terminate(target));
                        self.confirmation_needs_focus = true;
                    }
                    ui.menu_button("CPU affinity...", |ui| {
                        ui.label("Processor mask / hexadecimal");
                        ui.text_edit_singleline(&mut self.affinity_mask);
                        ui.label(
                            "Example: 1 = logical CPU 0; F = CPUs 0-3.
Single processor group only.",
                        );
                        if ui.button("Review affinity...").clicked() {
                            match usize::from_str_radix(
                                self.affinity_mask.trim().trim_start_matches("0x"),
                                16,
                            ) {
                                Ok(mask) if mask != 0 => {
                                    self.confirmation = Some(Action::Affinity(target, mask));
                                    self.confirmation_needs_focus = true;
                                    ui.close();
                                }
                                _ => {
                                    self.notice = Some((
                                        "Enter a nonzero hexadecimal processor mask.".into(),
                                        true,
                                    ))
                                }
                            }
                        }
                    });
                    ui.menu_button("Set priority...", |ui| {
                        for (level, text) in [
                            (PriorityClass::Idle, "Idle"),
                            (PriorityClass::BelowNormal, "Below normal"),
                            (PriorityClass::Normal, "Normal"),
                            (PriorityClass::AboveNormal, "Above normal"),
                        ] {
                            if ui.button(text).clicked() {
                                self.confirmation = Some(Action::Priority(target, level));
                                self.confirmation_needs_focus = true;
                                ui.close();
                            }
                        }
                    });
                });
            });
            if !enabled {
                ui.label(
                    RichText::new(
                        "Actions require a current live capture and an available target.",
                    )
                    .small()
                    .color(MUTED),
                );
            }
        } else {
            ui.label("Select a process to see available actions.");
        }
    }
    fn memory(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Save snapshot").clicked() {
                self.export_snapshot();
            }
            if ui.button("Open snapshot").clicked() {
                self.import_snapshot();
            }
            if self.historical.is_some() && ui.button("Return to live").clicked() {
                self.historical = None;
                self.selected = None;
            }
        });
        if let Some(s) = self.snapshot().cloned() {
            ui.columns(3, |c| {
                metric(
                    &mut c[0],
                    "IN USE",
                    &bytes(s.used_memory),
                    "System physical memory",
                    percent(s.used_memory, s.total_memory),
                    ACCENT,
                    self.preferences.reduced_motion,
                );
                metric(
                    &mut c[1],
                    "AVAILABLE",
                    &bytes(s.available_memory),
                    "Reusable memory, not just free pages",
                    percent(s.available_memory, s.total_memory),
                    GREEN,
                    self.preferences.reduced_motion,
                );
                metric(
                    &mut c[2],
                    "SWAP / PAGEFILE",
                    &bytes(s.used_swap),
                    &format!("{} configured", bytes(s.total_swap)),
                    percent(s.used_swap, s.total_swap),
                    CYAN,
                    self.preferences.reduced_motion,
                );
            });
        }
        ui.add_space(12.0);
        if self.historical.is_none()
            && let Some(m) = &self.live_memory
        {
            card(ui, |ui| {
                ui.strong("Windows accounting / live GetPerformanceInfo");
                egui::Grid::new("memory-counters")
                    .num_columns(2)
                    .spacing([24.0, 10.0])
                    .show(ui, |ui| {
                        for (label, value) in [
                            ("Commit", bytes(m.commit_bytes)),
                            ("Commit limit", bytes(m.commit_limit_bytes)),
                            ("System cache", bytes(m.system_cache_bytes)),
                            ("Paged kernel pool", bytes(m.kernel_paged_bytes)),
                            ("Nonpaged kernel pool", bytes(m.kernel_nonpaged_bytes)),
                            ("Handles", m.handles.to_string()),
                            ("Threads", m.threads.to_string()),
                        ] {
                            ui.label(label);
                            ui.monospace(value);
                            ui.end_row();
                        }
                    });
                ui.label(RichText::new("System cache includes overlapping accounting. Do not add it to physical usage. Exact standby/PFN inventory is unavailable.").small().color(MUTED));
            });
        }
        ui.add_space(12.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Process address mappings");
                if self.operation.is_some() {
                    ui.spinner();
                    ui.label("Reading Windows...");
                }
            });
            if let Some(t) = self.mappings_for {
                ui.label(format!(
                    "PID {} / captured regions: {} / virtual address space",
                    t.pid,
                    self.mappings.len()
                ));
                egui::ScrollArea::vertical()
                    .id_salt("mappings")
                    .max_height(260.0)
                    .show_rows(ui, 22.0, self.mappings.len(), |ui, range| {
                        for i in range {
                            let r = &self.mappings[i];
                            ui.monospace(format!(
                                "0x{:016X} {:>11} {:<12} {:<12} {}",
                                r.base,
                                bytes(r.size),
                                r.state,
                                r.kind,
                                r.protection
                            ));
                        }
                    });
            } else {
                ui.label("Choose a process in Task manager, then Inspect memory regions.");
            }
            if ui.button("Choose a process").clicked() {
                self.page = Page::Processes;
            }
            ui.label(RichText::new("Virtual mappings do not measure resident physical memory. Protected processes may deny access.").small().color(MUTED));
        });
        ui.add_space(12.0);
        ui.label(RichText::new(if crate::release::STORE { "Administration provides read-only services, startup and memory inventories." } else { "Manual system cache and advanced standby commands are in Administration. They require a fresh live capture, explicit elevation and confirmation. System-wide effects still need controlled-machine validation." }).color(MUTED));
    }
    fn workloads(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let paused = self.scheduler.paused;
            if ui
                .button(if paused {
                    "Resume queue"
                } else {
                    "Pause admission"
                })
                .clicked()
            {
                self.scheduler.set_paused(!paused);
            }
            ui.label(
                RichText::new("One owned workload / strict priority / 512 MiB reserve")
                    .color(MUTED),
            );
        });
        if let Some(error) = &self.scheduler.last_error {
            ui.colored_label(RED, error);
        }
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Built-in GPU pulse / native CUDA driver").default_open(true).show(ui,|ui|{ui.horizontal(|ui|{ui.label("VRAM MiB");ui.add(egui::DragValue::new(&mut self.pulse_mb).range(32..=2048));ui.label("Duration s");ui.add(egui::DragValue::new(&mut self.pulse_seconds).range(1..=3600));
if ui.button(RichText::new("Add GPU pulse").color(ACCENT)).clicked(){let result=self.scheduler.submit_self(format!("GPU pulse / {} MiB",self.pulse_mb),vec!["--gpu-pulse".into(),self.pulse_mb.to_string(),self.pulse_seconds.to_string()],Priority::Normal,self.pulse_mb);self.notice=Some(match result{Ok(id)=>(format!("Queued native GPU pulse #{id}."),false),Err(e)=>(e,true)});}});ui.label(RichText::new("Allocates VRAM and repeatedly fills it. Not an FPS benchmark. Requires an NVIDIA driver; no Python or CUDA Toolkit.").small().color(MUTED));});
        egui::CollapsingHeader::new("Submit a custom executable").show(ui, |ui| {
            egui::Grid::new("submit").num_columns(2).show(ui, |ui| {
                ui.label("Job name"); ui.text_edit_singleline(&mut self.job_name); ui.end_row();
                ui.label("Executable");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.executable).desired_width(380.0));
                    if ui.button("Browse").clicked() && let Some(path) = rfd::FileDialog::new().add_filter("Executable", &["exe"]).pick_file() {
                        self.executable = path.to_string_lossy().into_owned();
                    }
                });
                ui.end_row();
                ui.label("Arguments / JSON array");
                ui.add(egui::TextEdit::singleline(&mut self.arguments).desired_width(480.0).hint_text("[\"--flag\", \"path with spaces\"]"));
                ui.end_row();
                ui.label("Requested VRAM MiB"); ui.add(egui::DragValue::new(&mut self.requested_mb).range(0..=1048576)); ui.end_row();
                ui.label("Priority");
                egui::ComboBox::from_id_salt("job-priority").selected_text(format!("{:?}", self.priority)).show_ui(ui, |ui| {
                    for priority in [Priority::Critical, Priority::High, Priority::Normal, Priority::Low] { ui.selectable_value(&mut self.priority, priority, format!("{priority:?}")); }
                });
                ui.end_row();
            });
            if ui.button("Submit to queue").clicked() {
                let result = serde_json::from_str::<Vec<String>>(&self.arguments)
                    .map_err(|e| format!("Arguments must be a JSON string array: {e}"))
                    .and_then(|args| self.scheduler.submit(self.job_name.clone(), self.executable.clone(), args, self.priority, self.requested_mb));
                self.notice = Some(match result { Ok(id) => (format!("Workload #{id} queued."), false), Err(e) => (e, true) });
            }
            ui.label(RichText::new("Arguments pass directly without a shell. Declared VRAM is an estimate, not an enforced allocation limit.").small().color(MUTED));
        });
        ui.separator();
        let jobs = self.scheduler.jobs.clone();
        egui::ScrollArea::vertical()
            .id_salt("jobs")
            .max_height(250.0)
            .show(ui, |ui| {
                for job in jobs.iter().rev() {
                    card(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong(&job.name);
                            ui.colored_label(
                                match job.status {
                                    Status::Running => GREEN,
                                    Status::Failed | Status::Interrupted => RED,
                                    _ => MUTED,
                                },
                                format!("{:?}", job.status),
                            );
                            ui.label(format!(
                                "{:?} / {} MiB / #{}",
                                job.priority, job.required_vram_mb, job.id
                            ));
                            if matches!(
                                job.status,
                                Status::Queued | Status::Waiting | Status::Running
                            ) && ui.button("Cancel").clicked()
                                && let Err(e) = self.scheduler.cancel(job.id)
                            {
                                self.notice = Some((e, true));
                            }
                        });
                        ui.label(RichText::new(&job.reason).small().color(MUTED));
                    });
                }
                if jobs.is_empty() {
                    ui.label("Queue is empty. Add a native pulse or your own executable.");
                }
            });
        ui.separator();
        ui.strong("Decision feed");
        egui::ScrollArea::vertical()
            .id_salt("events")
            .max_height(190.0)
            .show(ui, |ui| {
                for event in self.scheduler.events.iter().rev().take(50) {
                    ui.label(RichText::new(event).small().color(MUTED));
                }
            });
    }
    fn policy(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Total VRAM MiB");
            ui.add(egui::DragValue::new(&mut self.policy_capacity).range(512..=65536));
            ui.label("Simulated slots");
            ui.add(egui::DragValue::new(&mut self.policy_slots).range(1..=8));
            ui.label("512 MiB reserve / virtual time");
        });
        for job in &mut self.policy_jobs {
            ui.horizontal(|ui| {
                ui.label(&job.name);
                ui.label("MiB");
                ui.add(egui::DragValue::new(&mut job.memory_mb).range(0..=65536));
                ui.label("seconds");
                ui.add(egui::DragValue::new(&mut job.duration_s).range(1..=300));
            });
        }
        ui.add_space(14.0);
        ui.columns(2, |columns| {
            for (index, policy) in [policy::Policy::Strict, policy::Policy::Fit]
                .into_iter()
                .enumerate()
            {
                card(&mut columns[index], |ui| {
                    ui.strong(match policy {
                        policy::Policy::Strict => "Strict priority",
                        policy::Policy::Fit => "Fit what fits",
                    });
                    match policy::simulate(
                        &self.policy_jobs,
                        self.policy_capacity,
                        512,
                        self.policy_slots,
                        policy,
                    ) {
                        Ok(result) => {
                            ui.label(format!(
                                "Makespan {}s / avg wait {:.1}s / slot use {:.1}%",
                                result.makespan_s,
                                result.average_wait_s,
                                result.utilization_percent
                            ));
                            let (rect, _) = ui.allocate_exact_size(
                                Vec2::new(
                                    ui.available_width(),
                                    self.policy_slots as f32 * 38.0 + 12.0,
                                ),
                                egui::Sense::hover(),
                            );
                            for (i, r) in result.records.iter().enumerate() {
                                let w = rect.width() / result.makespan_s.max(1) as f32;
                                let bar = egui::Rect::from_min_max(
                                    egui::pos2(
                                        rect.left() + r.start_s as f32 * w,
                                        rect.top() + r.slot as f32 * 38.0,
                                    ),
                                    egui::pos2(
                                        rect.left() + r.end_s as f32 * w,
                                        rect.top() + r.slot as f32 * 38.0 + 30.0,
                                    ),
                                );
                                ui.painter().rect_filled(
                                    bar,
                                    5.0,
                                    [ACCENT, CYAN, GREEN][i % 3].gamma_multiply(0.6),
                                );
                                ui.painter().text(
                                    bar.center(),
                                    egui::Align2::CENTER_CENTER,
                                    truncate(&r.name, 18),
                                    egui::FontId::proportional(11.0),
                                    Color32::WHITE,
                                );
                            }
                            for r in &result.records {
                                ui.label(format!(
                                    "{}: {}-{}s / waited {}s",
                                    r.name, r.start_s, r.end_s, r.wait_s
                                ));
                            }
                            for r in &result.rejected {
                                ui.colored_label(RED, format!("Rejected: {r}"));
                            }
                        }
                        Err(e) => {
                            ui.colored_label(RED, e);
                        }
                    }
                });
            }
        });
        ui.add_space(12.0);
        ui.label(RichText::new("Fit what fits can fill idle slots but lets smaller jobs bypass larger waiting jobs. Live admission uses strict priority with one slot.").color(MUTED));
    }
    fn settings(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.strong("Observation and motion");
            let mut changed = ui
                .checkbox(
                    &mut self.preferences.reduced_motion,
                    "Reduce motion / immediate metric changes",
                )
                .changed();
            ui.horizontal(|ui| {
                ui.label("Capture interval");
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.preferences.interval_seconds, 1..=10)
                            .suffix(" s"),
                    )
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label("Timeline window");
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.preferences.history_seconds, 30..=600)
                            .suffix(" s"),
                    )
                    .changed();
            });
            if changed {
                self.worker
                    .set_interval(Duration::from_secs(self.preferences.interval_seconds));
                self.notice = Some(
                    match serde_json::to_vec_pretty(&self.preferences)
                        .map_err(|e| e.to_string())
                        .and_then(|b| {
                            storage::atomic_write(&self.data_dir.join("preferences.json"), &b)
                        }) {
                        Ok(()) => ("Settings saved.".into(), false),
                        Err(e) => (e, true),
                    },
                );
            }
            ui.label(RichText::new("Capture runs on a separate worker. Animation frames never trigger resource collection.").small().color(MUTED));
        });
        ui.add_space(14.0);
        card(ui, |ui| {
            ui.strong("About VAnta");
            ui.label(format!("Edition: {}", crate::release::EDITION));
            ui.label(format!(
                "Rust {} / eframe + OpenGL / no Python or embedded browser",
                env!("CARGO_PKG_VERSION")
            ));
            ui.label("Native: telemetry, process actions, mappings, snapshots, CSV export, GPU pulse, strict queue, Policy Lab, history and motion settings.");
            ui.label("Still in Python: legacy SQLite import, long-term task usage history, per-process WDDM GPU counters and multi-adapter performance pages. Administration features depend on the edition.");
            ui.label("Native storage is isolated. Interrupted jobs require explicit queue resume.");
            if ui.button("Open application data folder").clicked() {
                let _ = std::process::Command::new("explorer.exe")
                    .arg(&self.data_dir)
                    .spawn();
            }
        });
        ui.add_space(14.0);
        card(ui, |ui| {
            egui::CollapsingHeader::new("Privacy and local data").show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("privacy-policy")
                    .max_height(280.0)
                    .show(ui, |ui| {
                        for paragraph in include_str!("../../docs/PRIVACY.md").split("\n\n") {
                            ui.label(paragraph.trim_start_matches('#').trim());
                            ui.add_space(4.0);
                        }
                    });
            });
        });
        ui.add_space(14.0);
        card(ui, |ui| {
            ui.strong("Keyboard");
            ui.label("Ctrl+1 overview / Ctrl+2 workloads / Ctrl+3 processes / Ctrl+4 memory / Ctrl+5 policy");
            ui.label("Ctrl+F process search / Ctrl+E snapshot export / Escape cancel confirmation");
        });
    }
    fn confirmation(&mut self, ctx: &egui::Context) {
        let Some(action) = self.confirmation.clone() else {
            return;
        };
        let target = action.target();
        let mut execute = false;
        let mut cancel = false;
        if self.confirmation_needs_focus {
            self.worker.set_paused(true);
        }
        egui::Modal::new(egui::Id::new("process-action-confirmation")).show(ctx, |ui| {
            ui.set_max_width(480.0);
            ui.heading(action.label());
            ui.label(format!("Target PID {} / process start {}", target.pid, target.start_time));
            if let Some(name) = self.live.as_ref().and_then(|s| s.processes.iter().find(|p| p.pid == target.pid && p.start_time == target.start_time)).map(|p| p.name.as_str()) { ui.strong(name); }
            match action {
                Action::Terminate(_) => { ui.colored_label(RED, "Ending this process can lose unsaved work. Only the selected process is targeted."); },
                Action::Trim(_) => { ui.label("Pages may leave the working set and fault back in. Private committed memory is not deallocated. Faster performance is not guaranteed."); },
                Action::Priority(_, level) => { ui.label(format!("Requested CPU priority: {level:?}. Permissions may prevent the action.")); },
                Action::Affinity(_, mask) => { ui.label(format!("Restrict to processor mask {mask:#x}. Only a single processor group is supported. Windows checks the allowed mask.")); },
                Action::Inspect(_) => {},
            }
            ui.label("The full creation timestamp is rechecked on the opened handle before any change.");
            ui.horizontal(|ui| {
                let response = ui.button("Cancel");
                if self.confirmation_needs_focus { response.request_focus(); self.confirmation_needs_focus = false; }
                if response.clicked() { cancel = true; }
                if ui.button(RichText::new("Confirm action").color(RED)).clicked() { execute = true; }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        if cancel || execute {
            self.confirmation = None;
            self.worker.set_paused(self.paused);
        }
        if execute && !cancel {
            self.start_action(action);
        }
    }
}
impl eframe::App for VantaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive();
        ctx.style_mut(|s| {
            s.animation_time = if self.preferences.reduced_motion {
                0.0
            } else {
                0.18
            }
        });
        if self.smoke {
            let seconds = self.started.elapsed().as_secs();
            self.page = match seconds {
                0..=1 => Page::Overview,
                2 => Page::Processes,
                3 => Page::Memory,
                4 => Page::Policy,
                5 => Page::Workloads,
                6 => Page::Administration,
                _ => Page::Settings,
            };
            if seconds >= 8 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        ctx.input(|input| {
            if input.modifiers.ctrl {
                for (key, page) in [
                    (egui::Key::Num1, Page::Overview),
                    (egui::Key::Num2, Page::Workloads),
                    (egui::Key::Num3, Page::Processes),
                    (egui::Key::Num4, Page::Memory),
                    (egui::Key::Num5, Page::Policy),
                    (egui::Key::Num6, Page::Administration),
                ] {
                    if input.key_pressed(key) {
                        self.page = page;
                    }
                }
                if input.key_pressed(egui::Key::F) {
                    self.page = Page::Processes;
                }
            }
        });
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::E)) {
            self.export_snapshot();
        }
        egui::SidePanel::left("sidebar")
            .exact_width(208.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(13, 15, 23))
                    .inner_margin(18.0),
            )
            .show(ctx, |ui| {
                ui.add_space(12.0);
                let logo_size = (ui.available_height() - 520.0).clamp(96.0, 160.0);
                ui.vertical_centered(|ui| {
                    ui.add(
                        egui::Image::new(&self.logo)
                            .fit_to_exact_size(Vec2::splat(logo_size))
                            .alt_text("Vanta Native logo"),
                    );
                });
                ui.label(
                    RichText::new("RESOURCE OBSERVATORY")
                        .size(10.0)
                        .color(MUTED),
                );
                ui.add_space(32.0);
                for page in [
                    Page::Overview,
                    Page::Workloads,
                    Page::Processes,
                    Page::Memory,
                    Page::Policy,
                    Page::Administration,
                ] {
                    let selected = self.page == page;
                    let text = RichText::new(page.title())
                        .color(if selected { ACCENT } else { MUTED })
                        .size(15.0);
                    if ui
                        .add_sized(
                            [ui.available_width(), 42.0],
                            egui::Button::new(text).selected(selected),
                        )
                        .clicked()
                    {
                        self.page = page;
                    }
                }
                let show_workflow = ui.available_height() >= 160.0;
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(RichText::new("0.2 / RUST NATIVE").size(10.0).color(MUTED));
                    if ui.button("Settings").clicked() {
                        self.page = Page::Settings;
                    }
                    ui.separator();
                    if show_workflow {
                        ui.label(
                            RichText::new("Observe\nRule\nQueue / action\nVerify")
                                .color(MUTED)
                                .size(13.0),
                        );
                    }
                });
            });
        egui::TopBottomPanel::top("topbar")
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(28, 16)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let historical = self.historical.is_some();
                    ui.colored_label(
                        if historical {
                            ACCENT
                        } else if self.paused {
                            MUTED
                        } else {
                            GREEN
                        },
                        if historical {
                            "HISTORICAL SNAPSHOT"
                        } else if self.paused {
                            "OBSERVATION PAUSED"
                        } else {
                            "LIVE OBSERVATIONS"
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(if self.paused {
                                "Resume capture"
                            } else {
                                "Pause capture"
                            })
                            .clicked()
                        {
                            self.paused = !self.paused;
                            self.worker.set_paused(self.paused);
                        }
                        if historical && ui.button("Return to live").clicked() {
                            self.historical = None;
                            self.selected = None;
                            self.mappings.clear();
                            self.mappings_for = None;
                        }
                        ui.label(
                            RichText::new(format!(
                                "{}s interval",
                                self.preferences.interval_seconds
                            ))
                            .color(MUTED),
                        );
                    });
                });
            });
        if self.admin.has_confirmation() {
            self.page = Page::Administration;
        }
        if self.displayed_page != self.page {
            self.displayed_page = self.page;
            self.transition_at = Instant::now();
        }
        let entry_opacity = if self.preferences.reduced_motion {
            1.0
        } else {
            (self.transition_at.elapsed().as_secs_f32() / 0.18).clamp(0.0, 1.0)
        };
        if entry_opacity < 1.0 {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(28.0))
            .show(ctx, |ui| {
                ui.set_opacity(entry_opacity);
                ui.heading(self.page.title());
                ui.label(RichText::new(self.page.subtitle()).color(MUTED));
                ui.add_space(16.0);
                if let Some((message, error)) = self.notice.clone() {
                    egui::Frame::new()
                        .fill(if error {
                            Color32::from_rgb(47, 24, 33)
                        } else {
                            Color32::from_rgb(22, 38, 36)
                        })
                        .corner_radius(8.0)
                        .inner_margin(12.0)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.colored_label(if error { RED } else { GREEN }, message);
                                if ui.small_button("Dismiss").clicked() {
                                    self.notice = None;
                                }
                            });
                        });
                    ui.add_space(12.0);
                }
                egui::ScrollArea::vertical()
                    .id_salt("page-scroll")
                    .show(ui, |ui| match self.page {
                        Page::Overview => self.overview(ui),
                        Page::Workloads => self.workloads(ui),
                        Page::Processes => self.processes(ui),
                        Page::Memory => self.memory(ui),
                        Page::Policy => self.policy(ui),
                        Page::Administration => {
                            if self.historical.is_some() { ui.colored_label(MUTED, "Administration is disabled while a historical snapshot is open. Return to live first."); }
                            ui.add_enabled_ui(self.historical.is_none() && self.operation.is_none() && self.confirmation.is_none(), |ui| self.admin.show(ui));
                        },
                        Page::Settings => self.settings(ui),
                    });
            });
        self.confirmation(ctx);
        ctx.request_repaint_after(Duration::from_millis(if self.paused { 500 } else { 250 }));
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.scheduler.shutdown();
        self.admin.shutdown();
        if let Some(mut op) = self.operation.take()
            && let Some(handle) = op.handle.take()
        {
            let _ = handle.join();
        }
    }
}
fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(35, 40, 57)))
        .corner_radius(11.0)
        .inner_margin(16.0)
        .show(ui, add)
        .inner
}
fn metric(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    detail: &str,
    pct: f32,
    color: Color32,
    reduced: bool,
) {
    ui.push_id(label, |ui| {
        card(ui, |ui| {
            ui.set_min_height(112.0);
            ui.label(RichText::new(label).color(MUTED).size(11.0));
            ui.label(RichText::new(value).size(28.0).strong().color(color));
            ui.label(RichText::new(detail).small().color(MUTED));
            let target = (pct / 100.0).clamp(0.0, 1.0);
            let value = if reduced {
                target
            } else {
                ui.ctx()
                    .animate_value_with_time(ui.id().with("metric-value"), target, 0.3)
            };
            ui.add(
                egui::ProgressBar::new(value)
                    .fill(color)
                    .desired_height(4.0),
            );
        });
    });
}
fn timeline(ui: &mut egui::Ui, history: &VecDeque<(u64, f32, f32, Option<f32>)>, seconds: u64) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 170.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    for fraction in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let y = rect.bottom() - rect.height() * fraction;
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            Stroke::new(1.0_f32, Color32::from_rgb(36, 41, 58)),
        );
    }
    let Some(last) = history.back() else {
        return;
    };
    for (series, color) in [(0, CYAN), (1, ACCENT), (2, GREEN)] {
        let mut previous = None;
        for sample in history {
            let value = match series {
                0 => Some(sample.1),
                1 => Some(sample.2),
                _ => sample.3,
            };
            if let Some(value) = value {
                let x = rect.right()
                    - last.0.saturating_sub(sample.0) as f32 / seconds.max(1) as f32 * rect.width();
                let point = egui::pos2(
                    x,
                    rect.bottom() - value.clamp(0.0, 100.0) / 100.0 * rect.height(),
                );
                if let Some(previous) = previous {
                    painter.line_segment([previous, point], Stroke::new(2.0_f32, color));
                }
                previous = Some(point);
            } else {
                previous = None;
            }
        }
    }
    painter.text(
        rect.left_top() + Vec2::new(5.0, 5.0),
        egui::Align2::LEFT_TOP,
        "100%",
        egui::FontId::monospace(10.0),
        MUTED,
    );
}
fn percent(value: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (value as f64 / total as f64 * 100.0).clamp(0.0, 100.0) as f32
    }
}
fn bytes(value: u64) -> String {
    if value >= 1073741824 {
        format!("{:.2} GiB", value as f64 / 1073741824.0)
    } else if value >= 1048576 {
        format!("{:.1} MiB", value as f64 / 1048576.0)
    } else if value >= 1024 {
        format!("{:.1} KiB", value as f64 / 1024.0)
    } else {
        format!("{value} B")
    }
}
fn truncate(value: &str, length: usize) -> String {
    let mut result: String = value.chars().take(length).collect();
    if value.chars().count() > length {
        result.push('\u{2026}');
    }
    result
}
fn epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn csv_cell(value: &str) -> String {
    let mut value = value.replace('"', "\"\"");
    if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        value.insert(0, '\'');
    }
    format!("\"{value}\"")
}

#[cfg(windows)]
fn system_reduced_motion() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    let mut enabled: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes a BOOL to this valid pointer; no setting is changed.
    let success = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        )
    };
    success != 0 && enabled == 0
}
#[cfg(not(windows))]
fn system_reduced_motion() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_process_names_cannot_become_spreadsheet_formulas() {
        assert_eq!(csv_cell("=cmd|x"), "\"'=cmd|x\"");
        assert_eq!(csv_cell("a\"b"), "\"a\"\"b\"");
    }
    #[test]
    fn unicode_process_names_truncate_at_character_boundaries() {
        assert_eq!(
            truncate("\u{e17}\u{e14}\u{e2a}\u{e2d}\u{e1a}", 3),
            "\u{e17}\u{e14}\u{e2a}\u{2026}"
        );
    }
}
