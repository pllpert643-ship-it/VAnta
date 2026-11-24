"""Task Manager views backed by a dedicated system-collection thread."""
from __future__ import annotations
from datetime import datetime
import os
from pathlib import Path
import subprocess
from urllib.parse import quote
import psutil
from PySide6.QtCore import QObject, QSettings, Qt, QThread, QTimer, QUrl, Signal, Slot
from PySide6.QtGui import QDesktopServices
from PySide6.QtWidgets import (QApplication, QCheckBox, QComboBox, QDialog, QDialogButtonBox,
    QFileDialog, QFormLayout, QHBoxLayout, QHeaderView, QLabel, QLineEdit, QMenu,
    QMessageBox, QPlainTextEdit, QPushButton, QScrollArea, QTabWidget, QTableWidget,
    QTableWidgetItem, QVBoxLayout, QWidget)
from vanta.infra.task_manager import ProcessRow, TaskManager
from vanta.infra.startup import StartupManager
from vanta.infra import windows_tasks as windows
from vanta.ui.widgets import TrendChart


def size(value):
    if value is None:
        return "Unavailable"
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if abs(value) < 1024 or unit == "TB":
            return f"{value:,.1f} {unit}"
        value /= 1024


def duration(seconds):
    seconds = int(seconds)
    return f"{seconds // 3600}:{seconds // 60 % 60:02}:{seconds % 60:02}"


class NumericItem(QTableWidgetItem):
    def __lt__(self, other):
        a, b = self.data(Qt.ItemDataRole.UserRole + 1), other.data(Qt.ItemDataRole.UserRole + 1)
        if isinstance(a, (float, int)) and isinstance(b, (float, int)):
            return a < b
        return self.text().casefold() < other.text().casefold()


class Collector(QObject):
    ready = Signal()
    sampled = Signal(object)
    result = Signal(str, object)
    failed = Signal(str)
    stopped = Signal()

    def __init__(self, data_dir):
        super().__init__()
        self.data_dir = data_dir
        self.manager = None
        self.com_initialized = False
        self.priorities = {}

    @Slot()
    def initialize(self):
        try:
            if os.name == "nt":
                import ctypes
                self.com_initialized = ctypes.windll.ole32.CoInitializeEx(None, 2) in (0, 1)
            self.manager = TaskManager(self.data_dir / "task_history.sqlite")
            self.startup = StartupManager(self.data_dir)
            self.ready.emit()
        except Exception as exc:
            self.failed.emit(f"Collector initialization: {exc}")

    @Slot(str)
    def sample(self, section):
        try:
            snap = self.manager.sample()
            extra = None
            if section == "App history":
                extra = self.manager.history()
            elif section == "Startup apps":
                extra = self.startup.entries()
            elif section == "Users":
                extra = windows.user_sessions()
            elif section == "Services":
                extra = self.manager.services()
            visible = windows.visible_processes() if os.name == "nt" else set()
            self.sampled.emit((snap, section, extra, visible))
        except Exception as exc:
            self.failed.emit(str(exc) or type(exc).__name__)

    @Slot(str, object)
    def operate(self, operation, payload):
        try:
            result = "Done"
            if operation == "end":
                self.manager.end_process(*payload)
                result = "Termination requested; verify the next refresh."
            elif operation == "priority":
                self.manager.set_priority(*payload)
            elif operation == "affinity":
                self.manager.set_affinity(*payload)
            elif operation == "affinity_read":
                result = self.manager.process(payload).cpu_affinity()
            elif operation == "efficiency":
                identity, enabled = payload
                proc = self.manager.process(identity, changing=True)
                old = proc.nice()
                windows.set_efficiency(identity, enabled)
                try:
                    self.manager.process(identity, changing=True).nice(psutil.BELOW_NORMAL_PRIORITY_CLASS if enabled else self.priorities.get(identity, psutil.NORMAL_PRIORITY_CLASS))
                except Exception:
                    windows.set_efficiency(identity, not enabled)
                    raise
                if enabled:
                    self.priorities.setdefault(identity, old)
                else:
                    self.priorities.pop(identity, None)
            elif operation == "dump":
                identity, path = payload
                self.manager.process(identity)
                result = windows.create_process_dump(identity, path)
            elif operation == "wait":
                self.manager.process(payload)
                result = windows.wait_chain(payload)
            elif operation == "switch":
                self.manager.process(payload)
                windows.switch_to_process(payload)
            elif operation == "properties":
                windows.open_file_properties(payload)
            elif operation == "startup":
                self.startup.toggle(*payload)
            elif operation == "service":
                result = windows.service_action(*payload)
            elif operation == "session":
                windows.session_action(*payload)
            elif operation == "run":
                windows.run_task(*payload)
            elif operation == "reset_history":
                self.manager.reset_history()
            elif operation == "restart_explorer":
                self.manager.end_process(payload)
                windows.run_task(str(Path(os.environ["WINDIR"]) / "explorer.exe"))
            else:
                raise ValueError("Unknown action")
            self.result.emit(operation, result)
        except Exception as exc:
            message = str(exc) or type(exc).__name__
            if isinstance(exc, (psutil.AccessDenied, PermissionError)):
                message += "\nWindows denied access. Administrator rights may be required or the target may be protected."
            self.failed.emit(message)

    @Slot()
    def close(self):
        try:
            if self.manager:
                self.manager.close()
        finally:
            if self.com_initialized:
                import ctypes
                ctypes.windll.ole32.CoUninitialize()
            self.stopped.emit()


class TaskManagerWidget(QWidget):
    request_sample = Signal(str)
    request_action = Signal(str, object)
    request_close = Signal()
    PAGES = ["Processes", "Performance", "App history", "Startup apps", "Users", "Details", "Services", "Settings"]
    COLUMNS = ["Name", "PID", "Status", "CPU %", "Memory", "I/O read/s", "I/O write/s", "User", "Threads", "Handles", "CPU time", "Private bytes", "Read bytes", "Write bytes", "Parent PID", "Session", "Priority", "Executable", "Command line", "GPU %", "GPU engine", "Dedicated GPU memory", "Shared GPU memory"]

    def __init__(self, data_dir: Path, parent=None):
        super().__init__(parent)
        self.settings = QSettings("VAnta", "TaskManager")
        self.snapshot = None
        self.busy = self.action_busy = self.initialized = self.stopping = False
        self._pending_affinity = None
        self._build_ui()
        self.thread = QThread(self)
        self.worker = Collector(data_dir)
        self.worker.moveToThread(self.thread)
        self.thread.started.connect(self.worker.initialize)
        self.worker.ready.connect(self._ready)
        self.request_sample.connect(self.worker.sample)
        self.request_action.connect(self.worker.operate)
        self.request_close.connect(self.worker.close)
        self.worker.sampled.connect(self._render)
        self.worker.result.connect(self._result)
        self.worker.failed.connect(self._error)
        self.worker.stopped.connect(self.thread.quit, Qt.ConnectionType.DirectConnection)
        self.thread.finished.connect(self.worker.deleteLater)
        self.timer = QTimer(self)
        self.timer.timeout.connect(self.refresh)
        self._interval(self.speed.currentData())
        self.thread.start()

    def _table(self, columns):
        table = QTableWidget(0, len(columns))
        table.setHorizontalHeaderLabels(columns)
        table.setAlternatingRowColors(True)
        table.setSelectionBehavior(QTableWidget.SelectionBehavior.SelectRows)
        table.setSelectionMode(QTableWidget.SelectionMode.SingleSelection)
        table.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        table.verticalHeader().hide()
        table.horizontalHeader().setSectionResizeMode(QHeaderView.ResizeMode.Interactive)
        table.horizontalHeader().setStretchLastSection(True)
        table.setColumnWidth(0, 210)
        table.setSortingEnabled(True)
        return table

    @staticmethod
    def _button(layout, text, callback):
        button = QPushButton(text)
        button.clicked.connect(callback)
        layout.addWidget(button)
        return button

    @staticmethod
    def _note(layout, text):
        label = QLabel(text)
        label.setWordWrap(True)
        layout.addWidget(label)
        return label

    def _build_ui(self):
        layout = QVBoxLayout(self)
        toolbar = QHBoxLayout()
        title = QLabel("TASK MANAGER")
        title.setObjectName("sectionTitle")
        toolbar.addWidget(title)
        toolbar.addStretch()
        self._button(toolbar, "Run new task", self._run_task)
        self._button(toolbar, "Windows Task Manager", lambda: self._open("Taskmgr.exe"))
        self._button(toolbar, "Refresh now", lambda: self.refresh(True))
        layout.addLayout(toolbar)
        self.search = QLineEdit()
        self.search.setPlaceholderText("Filter name, PID, user, executable or command line...")
        self.search.textChanged.connect(self._filter)
        layout.addWidget(self.search)
        self.tabs = QTabWidget()
        self.pages = {}
        for name in self.PAGES:
            page = QWidget()
            page.setLayout(QVBoxLayout())
            self.pages[name] = page.layout()
            self.tabs.addTab(page, name)
        self.processes = self._table(["Group"] + self.COLUMNS)
        self.details = self._table(self.COLUMNS)
        self.processes.setColumnWidth(0, 155)
        self.processes.setColumnWidth(1, 220)
        self.processes.setColumnWidth(2, 75)
        self.processes.setColumnWidth(3, 140)
        self.details.setColumnWidth(1, 75)
        self.details.setColumnWidth(2, 140)
        self.processes.sortItems(1, Qt.SortOrder.AscendingOrder)
        self.details.sortItems(0, Qt.SortOrder.AscendingOrder)
        for table, page, default in ((self.processes, "Processes", 9), (self.details, "Details", 12)):
            table.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
            table.customContextMenuRequested.connect(lambda point, t=table: self._menu(t, point, True))
            header = table.horizontalHeader()
            header.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
            header.customContextMenuRequested.connect(lambda point, t=table: self._columns(t, point))
            for col in range(table.columnCount()):
                table.setColumnHidden(col, self.settings.value(f"{page}/hidden/{col}", col >= default, type=bool))
            self.pages[page].addWidget(table)
            row = QHBoxLayout()
            self._button(row, "End task", lambda checked=False, t=table: self._end_selected(t))
            self._button(row, "More actions...", lambda checked=False, t=table: self._menu(t, t.viewport().rect().center()))
            self._button(row, "Choose columns", lambda checked=False, t=table: self._columns(t, t.horizontalHeader().rect().center()))
            self._button(row, "Copy table", lambda checked=False, t=table: self._copy(t))
            row.addStretch()
            self.pages[page].addLayout(row)
        self._build_performance()
        h = self.pages["App history"]
        self._note(h, "CPU time and process I/O observed while VAnta runs. This is VAnta history, not Windows App History, network traffic or startup impact.")
        self.history_since = self._note(h, "")
        self.history = self._table(["Application", "CPU time", "Read bytes", "Write bytes", "Last observed"])
        h.addWidget(self.history)
        self._button(h, "Delete VAnta usage history", self._reset_history)
        s = self.pages["Startup apps"]
        self._note(s, "Run keys and Startup folders. VAnta backs up entries it disables. Windows-disabled entries and packaged startup tasks use Windows Startup apps. Startup impact is not collected.")
        self.startup = self._table(["Name", "Status", "Location", "Command"])
        s.addWidget(self.startup)
        row = QHBoxLayout()
        self._button(row, "Enable", lambda: self._startup(True))
        self._button(row, "Disable", lambda: self._startup(False))
        self._button(row, "Windows Startup apps", lambda: QDesktopServices.openUrl(QUrl("ms-settings:startupapps")))
        row.addStretch()
        s.addLayout(row)
        self.users = self._table(["User", "Session", "Status", "Station", "Processes", "CPU %", "Memory"])
        self.pages["Users"].addWidget(self.users)
        row = QHBoxLayout()
        self._button(row, "Show processes", self._user_processes)
        self._button(row, "Disconnect", lambda: self._session(False))
        self._button(row, "Sign out", lambda: self._session(True))
        row.addStretch()
        self.pages["Users"].addLayout(row)
        self.services = self._table(["Name", "Display name", "Status", "PID", "Start type", "Account", "Executable"])
        self.pages["Services"].addWidget(self.services)
        row = QHBoxLayout()
        for verb in ("start", "stop", "restart"):
            self._button(row, verb.title(), lambda checked=False, v=verb: self._service(v))
        self._button(row, "Go to process", self._service_process)
        self._button(row, "Open Services", lambda: self._open("services.msc"))
        row.addStretch()
        self.pages["Services"].addLayout(row)
        self._build_settings()
        layout.addWidget(self.tabs, 1)
        self.status = self._note(layout, "Starting background collector...")
        default = self.settings.value("default_page", "Processes")
        self.tabs.setCurrentIndex(self.PAGES.index(default) if default in self.PAGES else 0)
        self.tabs.currentChanged.connect(self._tab_changed)

    def _build_performance(self):
        layout = self.pages["Performance"]
        self.summary = self._note(layout, "Waiting for system sample...")
        self._note(layout, "CPU (purple) / RAM (teal): last 60 observations. Change interval or pause in Settings.")
        self.chart = TrendChart()
        self.chart.setMaximumHeight(210)
        layout.addWidget(self.chart)
        self.performance_tabs = QTabWidget()
        self.perf = {}
        for name, columns in (
            ("CPU", ["Metric", "Value"]), ("Memory", ["Metric", "Value"]),
            ("Disks", ["Device", "Read B/s", "Write B/s", "Read bytes", "Write bytes"]),
            ("Network", ["Adapter", "Receive B/s", "Send B/s", "Link Mbps", "Status", "Addresses"]),
            ("GPU", ["GPU", "Name", "Utilization %", "Memory used", "Memory total", "Temperature C", "Power W", "Driver"]),
            ("GPU engines", ["Engine (Windows LUID)", "Utilization %"]),
            ("Hardware", ["Category", "Device", "Property", "Value"]),
        ):
            table = self._table(columns)
            self.perf[name] = table
            self.performance_tabs.addTab(table, name)
        layout.addWidget(self.performance_tabs, 1)
        row = QHBoxLayout()
        self._button(row, "Resource Monitor", lambda: self._open("resmon.exe"))
        self._button(row, "System information", lambda: self._open("msinfo32.exe"))
        self._button(row, "Copy table", lambda: self._copy(self.performance_tabs.currentWidget()))
        row.addStretch()
        layout.addLayout(row)
        self._note(layout, "GPU engines and per-process GPU use Windows WDDM counters (LUID identifiers). Temperature/power use NVIDIA NVML. Per-process network, NPU and disk response time are not collected. Hardware lists installed CPU, RAM modules, GPU drivers and disks; some firmware fields may be unavailable.")

    def _build_settings(self):
        layout = self.pages["Settings"]
        form = QFormLayout()
        self.speed = QComboBox()
        for label, ms in (("High (1 second)", 1000), ("Normal (2 seconds)", 2000), ("Low (4 seconds)", 4000), ("Paused", 0)):
            self.speed.addItem(label, ms)
        self.speed.setCurrentIndex(max(0, self.speed.findData(self.settings.value("interval", 2000, type=int))))
        self.speed.currentIndexChanged.connect(lambda: self._interval(self.speed.currentData()))
        form.addRow("Update speed", self.speed)
        default = QComboBox()
        default.addItems(self.PAGES[:-1])
        default.setCurrentText(self.settings.value("default_page", "Processes"))
        default.currentTextChanged.connect(lambda v: self.settings.setValue("default_page", v))
        form.addRow("Default page", default)
        self.on_top = QCheckBox("Keep VAnta above other windows")
        self.on_top.setChecked(self.settings.value("on_top", False, type=bool))
        self.on_top.toggled.connect(self._on_top)
        form.addRow("Always on top", self.on_top)
        self.confirm_efficiency = QCheckBox("Ask before changing Efficiency mode")
        self.confirm_efficiency.setChecked(self.settings.value("confirm_efficiency", True, type=bool))
        self.confirm_efficiency.toggled.connect(lambda v: self.settings.setValue("confirm_efficiency", v))
        form.addRow("Efficiency mode", self.confirm_efficiency)
        layout.addLayout(form)
        self._note(layout, "Some actions require administrator rights. Windows-protected processes may remain inaccessible. VAnta protects itself and core Windows processes. Live kernel dumps remain available through the Windows Task Manager button.")
        layout.addStretch()

    def _ready(self):
        self.initialized = True
        self._on_top(self.on_top.isChecked())
        self.refresh(True)

    def _interval(self, ms):
        self.settings.setValue("interval", ms)
        if hasattr(self, "timer"):
            self.timer.stop()
            if ms:
                self.timer.start(ms)

    def _on_top(self, enabled):
        self.settings.setValue("on_top", enabled)
        window = self.window()
        shown = window.isVisible()
        window.setWindowFlag(Qt.WindowType.WindowStaysOnTopHint, enabled)
        if shown:
            window.show()

    def _tab_changed(self):
        self.search.clear()
        self.refresh(True)

    def refresh(self, force=False):
        if self.stopping or not self.initialized or self.busy or self.action_busy:
            return
        if not force and not self.isVisible():
            return
        self.busy = True
        self.request_sample.emit(self.tabs.tabText(self.tabs.currentIndex()))

    @staticmethod
    def _key(obj):
        if isinstance(obj, ProcessRow):
            return obj.identity
        if isinstance(obj, dict):
            return obj.get("id", obj.get("name", obj.get("Session")))
        if isinstance(obj, (tuple, list)):
            return obj[0] if obj else None
        return obj

    @staticmethod
    def _selected(table):
        row = table.currentRow()
        item = table.item(row, 0) if row >= 0 and table.selectionModel().hasSelection() else None
        return item.data(Qt.ItemDataRole.UserRole) if item else None

    def _fill(self, table, rows, objects=None, raw=None):
        key = self._key(self._selected(table))
        header = table.horizontalHeader()
        sort_column, order = header.sortIndicatorSection(), header.sortIndicatorOrder()
        scroll = table.verticalScrollBar().value()
        table.setUpdatesEnabled(False)
        table.setSortingEnabled(False)
        table.clearContents()
        table.setRowCount(len(rows))
        selected = None
        for r, row in enumerate(rows):
            for c, value in enumerate(row):
                item = NumericItem("Unavailable" if value is None else str(value))
                item.setData(Qt.ItemDataRole.UserRole + 1, raw[r][c] if raw is not None else value)
                item.setToolTip(item.text())
                if c == 0:
                    obj = objects[r] if objects is not None else None
                    item.setData(Qt.ItemDataRole.UserRole, obj)
                    if key is not None and self._key(obj) == key:
                        selected = item
                table.setItem(r, c, item)
        table.setSortingEnabled(True)
        table.sortItems(sort_column, order)
        if selected:
            table.setCurrentItem(selected)
            table.selectRow(selected.row())
        else:
            table.clearSelection()
            table.setCurrentCell(-1, -1)
        table.verticalScrollBar().setValue(scroll)
        table.setUpdatesEnabled(True)

    @Slot(object)
    def _render(self, result):
        self.busy = False
        if self.stopping:
            return
        snap, section, extra, visible = result
        self.snapshot = snap
        raw, display, groups = [], [], []
        windows_root = os.environ.get("WINDIR", "C:\\Windows").casefold() + "\\"
        for p in snap.processes:
            row = [p.name, p.pid, p.status + (" (restricted)" if not p.accessible else ""), p.cpu, p.memory,
                   p.read_rate, p.write_rate, p.user, p.threads, p.handles, p.cpu_time, p.private,
                   p.read_bytes, p.write_bytes, p.parent, p.session, p.priority, p.executable, p.command, p.gpu, p.gpu_engine, p.gpu_dedicated, p.gpu_shared]
            raw.append(row)
            formatted = row.copy()
            formatted[3] = round(p.cpu, 1)
            for c in (4, 5, 6, 11, 12, 13, 21, 22):
                formatted[c] = size(row[c]) + ("/s" if c in (5, 6) else "")
            formatted[19] = round(p.gpu, 1) if p.gpu is not None else None
            formatted[10] = duration(p.cpu_time)
            display.append(formatted)
            groups.append("App (visible window)" if p.pid in visible else "Windows directory" if p.executable.casefold().startswith(windows_root) else "Background")
        self._fill(self.details, display, snap.processes, raw)
        self._fill(self.processes, [[g] + r for g, r in zip(groups, display)], snap.processes, [[g] + r for g, r in zip(groups, raw)])
        self.chart.add_sample(snap.cpu_percent, snap.memory_percent)
        self.summary.setText(f"CPU {snap.cpu_percent:.1f}% (kernel {snap.cpu_kernel:.1f}%) | RAM {size(snap.memory_used)} / {size(snap.memory_total)} | Uptime {duration(snap.uptime)}")
        self._fill(self.perf["CPU"], list(snap.cpu_details.items()) + [(f"Logical processor {i}", f"{v:.1f}%") for i, v in enumerate(snap.cpu_per_core)])
        self._fill(self.perf["Memory"], [(k, f"{v}%" if k == "percent" else size(v)) for k, v in snap.memory_details.items()])
        for name, records in (("Disks", snap.disks), ("Network", snap.networks), ("GPU", snap.gpus), ("GPU engines", snap.gpu_engines)):
            table = self.perf[name]
            fields = [table.horizontalHeaderItem(c).text() for c in range(table.columnCount())]
            values = [[record.get(f) for f in fields] for record in records]
            formatted = [[size(v) if ("bytes" in f.lower() or "memory" in f.lower() or "B/s" in f) and isinstance(v, (int, float)) else round(v, 1) if isinstance(v, float) else v for f, v in zip(fields, row)] for row in values]
            self._fill(table, formatted, raw=values)
        hardware_rows = []
        for category, devices in snap.hardware.items():
            for index, device in enumerate(devices):
                for key, value in device.items():
                    hardware_rows.append([category, index, key, value])
        self._fill(self.perf["Hardware"], hardware_rows)
        current = self.tabs.tabText(self.tabs.currentIndex())
        if section == current:
            if section == "App history" and extra is not None:
                since, rows = extra
                self.history_since.setText("Observed since " + datetime.fromtimestamp(since).strftime("%Y-%m-%d %H:%M:%S"))
                self._fill(self.history, [[r[0], duration(r[1]), size(r[2]), size(r[3]), datetime.fromtimestamp(r[4]).strftime("%Y-%m-%d %H:%M:%S")] for r in rows], rows, rows)
            elif section == "Startup apps":
                self._fill(self.startup, [[e.get(k, "") for k in ("Name", "Status", "Location", "Command")] for e in extra or []], extra or [])
            elif section == "Services":
                self._fill(self.services, [[s.get(k, "") for k in ("name", "display_name", "status", "pid", "start_type", "username", "binpath")] for s in extra or []], extra or [])
            elif section == "Users":
                rows = []
                for session in extra or []:
                    processes = [p for p in snap.processes if p.session == session["Session"]]
                    rows.append([session["User"], session["Session"], session["Status"], session["Station"], len(processes), round(sum(p.cpu for p in processes), 1), size(sum(p.memory for p in processes))])
                self._fill(self.users, rows, extra or [])
        self.status.setText(f"Updated {datetime.fromtimestamp(snap.timestamp):%H:%M:%S} | {len(snap.processes)} processes | {sum(not p.accessible for p in snap.processes)} with restricted fields. CPU needs two observations. " + ("Paused." if not self.speed.currentData() else "") + (" GPU unavailable." if not snap.gpus else ""))
        self._filter()
        if section != current:
            self.refresh(True)

    def _filter(self):
        text = self.search.text().casefold()
        for table in (self.processes, self.details, self.history, self.startup, self.users, self.services):
            for row in range(table.rowCount()):
                content = " ".join(table.item(row, c).text() if table.item(row, c) else "" for c in range(table.columnCount()))
                table.setRowHidden(row, text not in content.casefold())

    def _ask(self, title, message):
        return QMessageBox.question(self, title, message, QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No, QMessageBox.StandardButton.No) == QMessageBox.StandardButton.Yes

    def _operate(self, action, payload):
        if self.action_busy or self.stopping or not self.initialized:
            return
        self.action_busy = True
        self.status.setText("Working...")
        self.request_action.emit(action, payload)

    def _end_selected(self, table):
        p = self._selected(table)
        if isinstance(p, ProcessRow):
            self._end(p, False)

    def _end(self, p, tree):
        if self._ask("End task", f"End {p.name} (PID {p.pid})" + (" and child processes" if tree else "") + "? Unsaved work may be lost."):
            self._operate("end", (p.identity, tree))

    def _menu(self, table, point, select=False):
        if select:
            item = table.itemAt(point)
            if item:
                table.setCurrentItem(item)
                table.selectRow(item.row())
        p = self._selected(table)
        if not isinstance(p, ProcessRow):
            self.status.setText("Select a process first.")
            return
        menu = QMenu(self)
        menu.addAction("End task", lambda: self._end(p, False))
        menu.addAction("End process tree", lambda: self._end(p, True))
        menu.addAction("Switch to window", lambda: self._operate("switch", p.identity))
        priorities = menu.addMenu("Set priority")
        for label, name in (("Idle", "IDLE"), ("Below normal", "BELOW_NORMAL"), ("Normal", "NORMAL"), ("Above normal", "ABOVE_NORMAL"), ("High", "HIGH"), ("Realtime", "REALTIME")):
            value = getattr(psutil, name + "_PRIORITY_CLASS", None)
            action = priorities.addAction(label, lambda checked=False, v=value, text=label: self._priority(p, v, text))
            action.setEnabled(value is not None)
        menu.addAction("Set affinity...", lambda: self._affinity(p))
        menu.addAction("Enable Efficiency mode", lambda: self._efficiency(p, True))
        menu.addAction("Disable Efficiency mode", lambda: self._efficiency(p, False))
        menu.addSeparator()
        menu.addAction("Analyze wait chain", lambda: self._operate("wait", p.identity))
        menu.addAction("Create process memory dump...", lambda: self._dump(p))
        menu.addSeparator()
        action = menu.addAction("Open file location", lambda: subprocess.Popen([str(Path(os.environ["WINDIR"]) / "explorer.exe"), "/select,", p.executable]))
        action.setEnabled(bool(p.executable))
        action = menu.addAction("Properties", lambda: self._operate("properties", p.executable))
        action.setEnabled(bool(p.executable))
        menu.addAction("Search online", lambda: QDesktopServices.openUrl(QUrl("https://www.bing.com/search?q=" + quote(p.name))))
        menu.addAction("Go to details", lambda: self._go_details(p.pid))
        menu.addAction("Go to services", lambda: self._go_services(p.pid))
        if p.name.lower() == "explorer.exe":
            menu.addAction("Restart Windows Explorer", lambda: self._restart_explorer(p))
        menu.exec(table.viewport().mapToGlobal(point))

    def _priority(self, p, value, text):
        if self._ask("Set priority", f"Set {p.name} ({p.pid}) to {text}?" + (" Realtime can make the system unresponsive." if text == "Realtime" else "")):
            self._operate("priority", (p.identity, value))

    def _affinity(self, p):
        self._pending_affinity = p
        self._operate("affinity_read", p.identity)

    def _show_affinity(self, cpus):
        p = self._pending_affinity
        if p is None:
            return
        dialog = QDialog(self)
        dialog.setWindowTitle(f"Affinity - {p.name} ({p.pid})")
        layout = QVBoxLayout(dialog)
        self._note(layout, "Choose logical processors. Windows processor groups may impose additional limits.")
        area = QScrollArea()
        area.setWidgetResizable(True)
        inner = QWidget()
        choices = QVBoxLayout(inner)
        checks = []
        for i in range(psutil.cpu_count() or 1):
            check = QCheckBox(f"CPU {i}")
            check.setChecked(i in cpus)
            choices.addWidget(check)
            checks.append(check)
        area.setWidget(inner)
        layout.addWidget(area)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        layout.addWidget(buttons)
        dialog.resize(470, 470)
        if dialog.exec() == QDialog.DialogCode.Accepted:
            selected = [i for i, check in enumerate(checks) if check.isChecked()]
            if selected:
                self._operate("affinity", (p.identity, selected))
            else:
                QMessageBox.warning(self, "Affinity", "Select at least one logical processor.")
        self._pending_affinity = None

    def _efficiency(self, p, enabled):
        if not self.confirm_efficiency.isChecked() or self._ask("Efficiency mode", f"{'Enable' if enabled else 'Disable'} Efficiency mode for {p.name} ({p.pid})? Disabling restores priority remembered in this session, or Normal if unknown."):
            self._operate("efficiency", (p.identity, enabled))

    def _dump(self, p):
        path, _ = QFileDialog.getSaveFileName(self, "Process dump (may contain private data)", f"{p.name}-{p.pid}.dmp", "Memory dump (*.dmp)")
        if path:
            self._operate("dump", (p.identity, path))

    def _restart_explorer(self, p):
        if self._ask("Restart Explorer", "Restart the desktop and file manager?"):
            self._operate("restart_explorer", p.identity)

    def _go_details(self, pid):
        self.tabs.setCurrentIndex(self.PAGES.index("Details"))
        self.search.setText(str(pid))
        for row in range(self.details.rowCount()):
            if self.details.item(row, 0).data(Qt.ItemDataRole.UserRole).pid == pid:
                self.details.selectRow(row)
                break

    def _go_services(self, pid):
        self.tabs.setCurrentIndex(self.PAGES.index("Services"))
        self.search.setText(str(pid))

    def _columns(self, table, point):
        menu = QMenu(self)
        page = "Processes" if table is self.processes else "Details"
        for col in range(table.columnCount()):
            action = menu.addAction(table.horizontalHeaderItem(col).text())
            action.setCheckable(True)
            action.setChecked(not table.isColumnHidden(col))
            action.setEnabled(col != 0)
            def toggle(checked, c=col):
                table.setColumnHidden(c, not checked)
                self.settings.setValue(f"{page}/hidden/{c}", not checked)
            action.toggled.connect(toggle)
        menu.exec(table.horizontalHeader().mapToGlobal(point))

    def _copy(self, table):
        columns = [c for c in range(table.columnCount()) if not table.isColumnHidden(c)]
        lines = ["\t".join(table.horizontalHeaderItem(c).text() for c in columns)]
        for r in range(table.rowCount()):
            if not table.isRowHidden(r):
                lines.append("\t".join(table.item(r, c).text() if table.item(r, c) else "" for c in columns))
        QApplication.clipboard().setText("\n".join(lines))
        self.status.setText("Visible table copied.")

    def _startup(self, enable):
        entry = self._selected(self.startup)
        if not entry:
            return
        if enable and entry["Status"] != "Disabled by VAnta":
            QMessageBox.information(self, "Startup apps", "Use Windows Startup apps for Windows-disabled entries. VAnta restores its own backups.")
            return
        if not enable and entry["Status"] != "Enabled":
            return
        if self._ask("Startup application", f"{'Enable' if enable else 'Disable'} {entry['Name']} for future sign-ins?"):
            self._operate("startup", (entry, enable))

    def _session(self, logoff):
        session = self._selected(self.users)
        if session and self._ask("Sign out" if logoff else "Disconnect", f"{'Sign out' if logoff else 'Disconnect'} {session['User']} (session {session['Session']})?" + (" Unsaved work will be lost." if logoff else "")):
            self._operate("session", (session["Session"], logoff))

    def _user_processes(self):
        session = self._selected(self.users)
        if session:
            self.tabs.setCurrentIndex(self.PAGES.index("Details"))
            self.search.setText(session["User"])

    def _service(self, action):
        service = self._selected(self.services)
        if service and self._ask(action.title() + " service", f"{action.title()} {service['display_name']} ({service['name']})?"):
            self._operate("service", (service["name"], action))

    def _service_process(self):
        service = self._selected(self.services)
        if service and service.get("pid"):
            self._go_details(service["pid"])

    def _run_task(self):
        dialog = QDialog(self)
        dialog.setWindowTitle("Run new task")
        layout = QFormLayout(dialog)
        command = QLineEdit()
        command.setPlaceholderText('notepad.exe or "C:\\path\\app.exe" arguments')
        elevated = QCheckBox("Request administrator privileges (Windows UAC)")
        layout.addRow("Command", command)
        layout.addRow(elevated)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        layout.addRow(buttons)
        dialog.resize(550, 150)
        if dialog.exec() == QDialog.DialogCode.Accepted and command.text().strip():
            self._operate("run", (command.text().strip(), elevated.isChecked()))

    def _reset_history(self):
        if self._ask("Delete history", "Delete usage history captured by VAnta?"):
            self._operate("reset_history", None)

    def _open(self, name):
        self._operate("run", (name, False))

    @Slot(str, object)
    def _result(self, operation, result):
        self.action_busy = False
        if self.stopping:
            return
        if operation == "affinity_read":
            self._show_affinity(result)
        elif operation == "wait":
            dialog = QDialog(self)
            dialog.setWindowTitle("Wait chain analysis")
            layout = QVBoxLayout(dialog)
            text = QPlainTextEdit(str(result))
            text.setReadOnly(True)
            layout.addWidget(text)
            dialog.resize(800, 500)
            dialog.exec()
        elif operation == "dump":
            QMessageBox.information(self, "Dump created", str(result))
        self.status.setText(f"{operation}: {result}")
        self.refresh(True)

    @Slot(str)
    def _error(self, message):
        self.busy = False
        was_action = self.action_busy
        self.action_busy = False
        if self.stopping:
            return
        self.status.setText(message)
        if was_action:
            QMessageBox.warning(self, "Action unavailable", message)

    def shutdown(self):
        if not self.stopping:
            self.stopping = True
            self.timer.stop()
            self.request_close.emit()
        return self.thread.wait(100)
