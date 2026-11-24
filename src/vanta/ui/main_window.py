from __future__ import annotations

import os
import shlex
import sys
from datetime import datetime

from PySide6.QtCore import Qt, QTimer
from PySide6.QtGui import QColor
from PySide6.QtWidgets import (
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QFrame,
    QGridLayout,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMainWindow,
    QMessageBox,
    QPushButton,
    QSpinBox,
    QTabWidget,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

from vanta.core.models import ActivityEvent, Job, JobStatus, Priority, ResourceSnapshot
from vanta.core.demo import build_demo_specs
from vanta.core.scheduler import Scheduler
from vanta.infra.monitor import SystemMonitor
from vanta.ui.policy_lab import PolicyLabDialog
from vanta.ui.widgets import FlowStep, MetricCard, TrendChart
from vanta.ui.task_manager import TaskManagerWidget
from vanta.ui.memory_map import MemoryMapWidget


STATUS_COLORS = {
    JobStatus.QUEUED: "#B8A7F4",
    JobStatus.WAITING: "#F6C177",
    JobStatus.RUNNING: "#75E6BB",
    JobStatus.COMPLETED: "#79D9F2",
    JobStatus.FAILED: "#FF8795",
    JobStatus.CANCELLED: "#9297A8",
}


def workload_command(memory_mb: int, duration: int) -> list[str]:
    if getattr(sys, "frozen", False):
        return [sys.executable, "--gpu-workload", "--memory-mb", str(memory_mb), "--duration", str(duration)]
    return [
        sys.executable,
        "-m",
        "vanta",
        "--gpu-workload",
        "--memory-mb",
        str(memory_mb),
        "--duration",
        str(duration),
    ]


class CustomJobDialog(QDialog):
    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setWindowTitle("Submit custom job")
        self.setMinimumWidth(520)
        form = QFormLayout(self)
        form.setContentsMargins(22, 22, 22, 22)
        form.setSpacing(12)
        self.name_input = QLineEdit("Custom GPU job")
        self.command_input = QLineEdit()
        self.command_input.setPlaceholderText('python "C:\\path\\to\\script.py"')
        self.priority_input = QComboBox()
        for priority in reversed(list(Priority)):
            self.priority_input.addItem(priority.label, priority)
        self.priority_input.setCurrentIndex(2)
        self.vram_input = QSpinBox()
        self.vram_input.setRange(0, 3072)
        self.vram_input.setValue(512)
        self.vram_input.setSuffix(" MB")
        form.addRow("Job name", self.name_input)
        form.addRow("Command", self.command_input)
        form.addRow("Priority", self.priority_input)
        form.addRow("Expected VRAM", self.vram_input)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Cancel | QDialogButtonBox.StandardButton.Ok
        )
        buttons.button(QDialogButtonBox.StandardButton.Ok).setText("Submit job")
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        form.addRow(buttons)

    def build_job(self) -> Job | None:
        command_text = self.command_input.text().strip()
        if not command_text:
            return None
        command = shlex.split(command_text, posix=os.name != "nt")
        if os.name == "nt":
            command = [part[1:-1] if len(part) >= 2 and part[0] == part[-1] == '"' else part for part in command]
        return Job(
            name=self.name_input.text().strip() or "Custom GPU job",
            command=command,
            priority=self.priority_input.currentData(),
            required_vram_mb=self.vram_input.value(),
        )


class MainWindow(QMainWindow):
    def __init__(self, monitor: SystemMonitor, scheduler: Scheduler) -> None:
        super().__init__()
        self.monitor = monitor
        self.scheduler = scheduler
        self.scheduler.event_sink = self.on_activity
        self.snapshot = ResourceSnapshot()
        self.setWindowTitle("VAnta — GPU Workload Controller")
        self.setMinimumSize(1100, 700)
        self.resize(1360, 850)
        self._build_ui()
        self._attach_task_manager()
        self._load_activity_history()

        self.timer = QTimer(self)
        self.timer.timeout.connect(self.refresh)
        self.timer.start(1000)
        self.refresh()

    def _attach_task_manager(self) -> None:
        from vanta.app import data_directory
        dashboard = self.takeCentralWidget()
        self.workspace_tabs = QTabWidget()
        self.workspace_tabs.setObjectName("workspaceTabs")
        self.workspace_tabs.addTab(dashboard, "GPU Workloads")
        self.task_manager = TaskManagerWidget(data_directory(), self)
        self.workspace_tabs.addTab(self.task_manager, "Task Manager")
        self.memory_map = MemoryMapWidget(self)
        self.workspace_tabs.addTab(self.memory_map, "Memory Map")
        self.workspace_tabs.setCurrentIndex(1)
        self.setCentralWidget(self.workspace_tabs)
        self.setWindowTitle("VAnta - System Control & GPU Workloads")

    def _build_ui(self) -> None:
        root = QWidget()
        root.setObjectName("root")
        self.setCentralWidget(root)
        outer = QVBoxLayout(root)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)
        outer.addWidget(self._build_header())

        content = QHBoxLayout()
        content.setContentsMargins(20, 18, 20, 20)
        content.setSpacing(16)
        left = QVBoxLayout()
        left.setSpacing(14)
        left.addLayout(self._build_metrics())
        left.addWidget(self._build_chart(), 1)
        left.addWidget(self._build_jobs(), 2)
        content.addLayout(left, 7)

        right = QVBoxLayout()
        right.setSpacing(14)
        right.addWidget(self._build_submit_card())
        right.addWidget(self._build_flow_card())
        right.addWidget(self._build_activity_card(), 1)
        content.addLayout(right, 3)
        outer.addLayout(content, 1)

    def _build_header(self) -> QFrame:
        frame = QFrame()
        frame.setObjectName("topBar")
        frame.setFixedHeight(72)
        row = QHBoxLayout(frame)
        row.setContentsMargins(22, 0, 22, 0)
        mark = QLabel("V")
        mark.setObjectName("brandMark")
        brand = QLabel("ANTA")
        brand.setObjectName("brand")
        tagline = QLabel("USER-LEVEL GPU WORKLOAD CONTROLLER")
        tagline.setObjectName("muted")
        self.device_label = QLabel("Detecting NVIDIA GPU…")
        self.device_label.setObjectName("muted")
        self.controller_label = QLabel("●  CONTROLLER LIVE")
        self.controller_label.setObjectName("livePill")
        row.addWidget(mark)
        row.addWidget(brand)
        row.addSpacing(10)
        row.addWidget(tagline)
        row.addStretch()
        row.addWidget(self.device_label)
        row.addSpacing(12)
        row.addWidget(self.controller_label)
        return frame

    def _build_metrics(self) -> QGridLayout:
        layout = QGridLayout()
        layout.setHorizontalSpacing(12)
        self.gpu_card = MetricCard("GPU Load", accent=True)
        self.vram_card = MetricCard("VRAM")
        self.cpu_card = MetricCard("CPU")
        self.ram_card = MetricCard("System RAM")
        for column, card in enumerate((self.gpu_card, self.vram_card, self.cpu_card, self.ram_card)):
            layout.addWidget(card, 0, column)
        return layout

    def _build_chart(self) -> QFrame:
        frame = QFrame()
        frame.setObjectName("card")
        layout = QVBoxLayout(frame)
        layout.setContentsMargins(16, 14, 16, 12)
        header = QHBoxLayout()
        title = QLabel("Live resource signal")
        title.setObjectName("sectionTitle")
        header.addWidget(title)
        header.addStretch()
        gpu_legend = QLabel("● GPU load")
        gpu_legend.setStyleSheet("color:#A98DFF;")
        vram_legend = QLabel("● VRAM used")
        vram_legend.setStyleSheet("color:#4DD8C7;")
        header.addWidget(gpu_legend)
        header.addWidget(vram_legend)
        self.chart = TrendChart()
        layout.addLayout(header)
        layout.addWidget(self.chart)
        return frame

    def _build_jobs(self) -> QFrame:
        frame = QFrame()
        frame.setObjectName("card")
        layout = QVBoxLayout(frame)
        layout.setContentsMargins(16, 14, 16, 12)
        header = QHBoxLayout()
        title = QLabel("Workload queue")
        title.setObjectName("sectionTitle")
        subtitle = QLabel("Priority ordered · non-preemptive")
        subtitle.setObjectName("muted")
        self.cancel_button = QPushButton("Cancel selected")
        self.cancel_button.setObjectName("dangerButton")
        self.cancel_button.clicked.connect(self.cancel_selected)
        header.addWidget(title)
        header.addWidget(subtitle)
        header.addStretch()
        header.addWidget(self.cancel_button)
        layout.addLayout(header)

        self.jobs_table = QTableWidget(0, 6)
        self.jobs_table.setHorizontalHeaderLabels(
            ["JOB", "PRIORITY", "STATUS", "VRAM", "PID", "DECISION"]
        )
        self.jobs_table.setAlternatingRowColors(True)
        self.jobs_table.setSelectionBehavior(QTableWidget.SelectionBehavior.SelectRows)
        self.jobs_table.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        self.jobs_table.verticalHeader().setVisible(False)
        header_view = self.jobs_table.horizontalHeader()
        header_view.setSectionResizeMode(0, QHeaderView.ResizeMode.ResizeToContents)
        header_view.setSectionResizeMode(1, QHeaderView.ResizeMode.ResizeToContents)
        header_view.setSectionResizeMode(2, QHeaderView.ResizeMode.ResizeToContents)
        header_view.setSectionResizeMode(3, QHeaderView.ResizeMode.ResizeToContents)
        header_view.setSectionResizeMode(4, QHeaderView.ResizeMode.ResizeToContents)
        header_view.setSectionResizeMode(5, QHeaderView.ResizeMode.Stretch)
        layout.addWidget(self.jobs_table)
        return frame

    def _build_submit_card(self) -> QFrame:
        frame = QFrame()
        frame.setObjectName("card")
        layout = QVBoxLayout(frame)
        layout.setContentsMargins(16, 15, 16, 16)
        layout.setSpacing(9)
        title = QLabel("Launch controlled workload")
        title.setObjectName("sectionTitle")
        description = QLabel("A safe CUDA pulse makes scheduling decisions visible.")
        description.setWordWrap(True)
        description.setObjectName("muted")
        self.preset = QComboBox()
        self.preset.addItem("Light pulse · 256 MB · 20 sec", (256, 20))
        self.preset.addItem("Balanced pulse · 512 MB · 30 sec", (512, 30))
        self.preset.addItem("Heavy pulse · 768 MB · 40 sec", (768, 40))
        self.priority = QComboBox()
        for priority in reversed(list(Priority)):
            self.priority.addItem(f"{priority.label} priority", priority)
        self.priority.setCurrentIndex(2)
        submit = QPushButton("Add GPU Pulse to queue")
        submit.setObjectName("primaryButton")
        submit.clicked.connect(self.submit_pulse)
        demo = QPushButton("Run demo scenario")
        demo.setObjectName("primaryButton")
        demo.setToolTip("Queue a normal job, a critical job, and a VRAM guard check")
        demo.clicked.connect(self.submit_demo)
        custom = QPushButton("Submit custom command")
        custom.clicked.connect(self.submit_custom)
        policy_lab = QPushButton("Open Policy Lab")
        policy_lab.setToolTip("Replay a workload scenario under two queue policies")
        policy_lab.clicked.connect(self.open_policy_lab)
        layout.addWidget(title)
        layout.addWidget(description)
        layout.addWidget(self.preset)
        layout.addWidget(self.priority)
        layout.addWidget(submit)
        layout.addWidget(demo)
        layout.addWidget(custom)
        layout.addWidget(policy_lab)
        return frame

    def _build_flow_card(self) -> QFrame:
        frame = QFrame()
        frame.setObjectName("card")
        layout = QVBoxLayout(frame)
        layout.setContentsMargins(16, 14, 16, 14)
        layout.setSpacing(3)
        title = QLabel("Controller loop")
        title.setObjectName("sectionTitle")
        layout.addWidget(title)
        layout.addWidget(FlowStep("1", "Observe", "GPU, VRAM, CPU and RAM"))
        layout.addWidget(FlowStep("2", "Rule", "1 job · 512 MB reserve"))
        layout.addWidget(FlowStep("3", "Queue / Action", "Strict priority then arrival time"))
        layout.addWidget(FlowStep("4", "Verify", "Measure after launch"))
        return frame

    def _build_activity_card(self) -> QFrame:
        frame = QFrame()
        frame.setObjectName("card")
        layout = QVBoxLayout(frame)
        layout.setContentsMargins(16, 14, 16, 12)
        title = QLabel("Decision feed")
        title.setObjectName("sectionTitle")
        self.activity_list = QListWidget()
        self.activity_list.setWordWrap(True)
        self.activity_list.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        layout.addWidget(title)
        layout.addWidget(self.activity_list)
        return frame

    def refresh(self) -> None:
        self.snapshot = self.monitor.sample()
        self.scheduler.tick(self.snapshot)
        self.controller_label.setText(
            "●  QUEUE PAUSED" if self.scheduler.recovery_blocked else "●  CONTROLLER LIVE"
        )
        self._update_metrics(self.snapshot)
        self._update_jobs()

    def _update_metrics(self, snapshot: ResourceSnapshot) -> None:
        self.gpu_card.update_value(
            f"{snapshot.gpu_utilization:.0f}%",
            f"{snapshot.temperature_c:.0f} °C" if snapshot.temperature_c is not None else "Temperature unavailable",
        )
        self.vram_card.update_value(
            f"{snapshot.vram_percent:.0f}%",
            f"{snapshot.vram_used_mb:.0f} / {snapshot.vram_total_mb:.0f} MB",
        )
        self.cpu_card.update_value(f"{snapshot.cpu_percent:.0f}%", "Host processor")
        self.ram_card.update_value(f"{snapshot.ram_percent:.0f}%", "System memory")
        self.device_label.setText(snapshot.gpu_name if snapshot.telemetry_available else "GPU telemetry unavailable")
        self.chart.add_sample(snapshot.gpu_utilization, snapshot.vram_percent)

    def _update_jobs(self) -> None:
        jobs = sorted(self.scheduler.jobs, key=lambda job: job.created_at, reverse=True)
        self.jobs_table.setRowCount(len(jobs))
        for row, job in enumerate(jobs):
            values = [
                job.name,
                job.priority.label,
                job.status.value.upper(),
                f"{job.required_vram_mb} MB",
                str(job.pid or "—"),
                job.status_reason,
            ]
            for column, value in enumerate(values):
                item = QTableWidgetItem(value)
                item.setData(Qt.ItemDataRole.UserRole, job.id)
                if column == 2:
                    item.setForeground(QColor(STATUS_COLORS[job.status]))
                self.jobs_table.setItem(row, column, item)

    def submit_pulse(self) -> None:
        memory_mb, duration = self.preset.currentData()
        priority = self.priority.currentData()
        sequence = len(self.scheduler.jobs) + 1
        job = Job(
            name=f"GPU Pulse {sequence:02d}",
            command=workload_command(memory_mb, duration),
            priority=priority,
            required_vram_mb=memory_mb,
        )
        self.scheduler.submit(job)
        self.refresh()

    def submit_demo(self) -> None:
        """Queue the complete presentation scenario in a predictable order."""
        specs = build_demo_specs(self.snapshot)
        sequence = len(self.scheduler.jobs) + 1

        # Start the normal workload before adding the critical workload. This
        # makes the non-preemptive rule visible instead of letting priority
        # ordering choose the first job before the demo begins.
        first = specs[0]
        self.scheduler.submit(
            Job(
                name=f"{first.name} {sequence:02d}",
                command=workload_command(first.pulse_memory_mb, first.duration_s),
                priority=first.priority,
                required_vram_mb=first.required_vram_mb,
            )
        )
        self.refresh()

        for offset, spec in enumerate(specs[1:], start=1):
            self.scheduler.submit(
                Job(
                    name=f"{spec.name} {sequence + offset:02d}",
                    command=workload_command(spec.pulse_memory_mb, spec.duration_s),
                    priority=spec.priority,
                    required_vram_mb=spec.required_vram_mb,
                )
            )
        self.refresh()

    def submit_custom(self) -> None:
        dialog = CustomJobDialog(self)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return
        try:
            job = dialog.build_job()
        except ValueError as exc:
            QMessageBox.warning(self, "Invalid command", str(exc))
            return
        if job is None:
            QMessageBox.warning(self, "Command required", "Enter the program or script to run.")
            return
        self.scheduler.submit(job)
        self.refresh()

    def cancel_selected(self) -> None:
        row = self.jobs_table.currentRow()
        if row < 0:
            return
        item = self.jobs_table.item(row, 0)
        if item is not None:
            self.scheduler.cancel(item.data(Qt.ItemDataRole.UserRole))
            self.refresh()

    def on_activity(self, event: ActivityEvent) -> None:
        local_time = datetime.fromisoformat(event.timestamp).astimezone().strftime("%H:%M:%S")
        icons = {
            "queue": "◇", "action": "▶", "verify": "✓", "complete": "●",
            "rule": "◆", "error": "!", "cancel": "×",
        }
        item = QListWidgetItem(f"{icons.get(event.kind, '·')}  {local_time}  {event.message}")
        item.setToolTip(event.message)
        self.activity_list.insertItem(0, item)
        while self.activity_list.count() > 50:
            self.activity_list.takeItem(self.activity_list.count() - 1)

    def _load_activity_history(self) -> None:
        events = self.scheduler.repository.recent_events(limit=50)
        # The repository returns newest first; inserting in reverse preserves
        # the same newest-at-top presentation as live events.
        for event in reversed(events):
            timestamp = str(event.get("timestamp", ""))
            try:
                local_time = datetime.fromisoformat(timestamp).astimezone().strftime("%H:%M:%S")
            except ValueError:
                local_time = "--:--:--"
            item = QListWidgetItem(
                f"•  {local_time}  {event.get('message', '')}"
            )
            item.setToolTip(str(event.get("message", "")))
            self.activity_list.insertItem(0, item)

    def open_policy_lab(self) -> None:
        PolicyLabDialog(self).exec()

    def closeEvent(self, event: object) -> None:  # noqa: N802 - Qt API
        # Stop admitting GPU work immediately, even while a native read finishes.
        if not getattr(self, "_shutdown_started", False):
            self._shutdown_started = True
            self.timer.stop()
            for job in list(self.scheduler.jobs):
                if job.status in {JobStatus.QUEUED, JobStatus.WAITING, JobStatus.RUNNING}:
                    self.scheduler.cancel(job.id)
        task_stopped = self.task_manager.shutdown()
        memory_stopped = self.memory_map.shutdown()
        if not (task_stopped and memory_stopped):
            event.ignore()
            self.statusBar().showMessage("Finishing system collection before closing...")
            QTimer.singleShot(200, self.close)
            return
        self.timer.stop()
        for job in list(self.scheduler.jobs):
            if job.status in {JobStatus.QUEUED, JobStatus.WAITING, JobStatus.RUNNING}:
                self.scheduler.cancel(job.id)
        self.scheduler.runner.close()
        self.monitor.close()
        self.scheduler.repository.close()
        event.accept()
