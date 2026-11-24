"""RAM diagnostics and explicit memory maintenance; not full RAMMap parity."""
from __future__ import annotations

from PySide6.QtCore import QObject, Qt, QThread, QTimer, Signal, Slot
from PySide6.QtWidgets import (QAbstractItemView, QComboBox, QFileDialog, QHBoxLayout,
    QHeaderView, QLabel, QLineEdit, QMessageBox, QPlainTextEdit, QPushButton, QTabWidget,
    QTableWidget, QTableWidgetItem, QVBoxLayout, QWidget)


CAPABILITIES = """VAnta Memory Map — scope and data definitions

This is an independently implemented RAM analysis workspace with explicit maintenance inspired by RAMMap's diagnostic workflow. It does not embed, redistribute or invoke RAMMap.

Overview: physical RAM and commit/cache/kernel counters where supported. Available RAM is not identical to the free page list. Committed bytes are virtual-memory commitments, not physical RAM consumed. Windows GetPerformanceInfo SystemCache includes the system working set plus standby pages; it is NOT an exact standby measurement. Values from different accounting domains overlap and must not be added together.

Processes: working set (resident pages, potentially shared) and private bytes (private committed virtual memory). Private bytes are NOT private resident RAM. Summing working sets double-counts shared pages. Restricted/exited processes remain explicitly unavailable.

Process mappings: on-demand psutil memory mappings of one selected process, checked by PID AND creation time. This is not a physical-page inventory or RAMMap File Summary. Some Windows mapping sizes or resident counts are not exposed and show Unavailable. A zero supplied by Windows is not a recovered allocation size.

Not implemented: RAMMap PFN Use Counts, exact Active/Standby/Modified/Zeroed/Free lists, priority summary, physical pages/ranges, global file summary/file details and .RMP import/export. These require additional privileged/platform-specific evidence and are not synthesized from aggregate metrics.

Maintenance is manual only: trim one selected process, trim system file-cache working set, or advanced standby purge. Standby purge uses an undocumented Windows native interface and is not guaranteed across Windows versions. Administrator privileges must be supplied by explicitly launching VAnta as administrator; there is no automatic elevation or driver loading. No all-process trim or modified-list write is provided. Every operation requires confirmation; cancellation does nothing. Trimming/purging does not fix leaks or guarantee better performance and can cause page faults or disk activity. Before/after available RAM is an observation, not attributable freed bytes. Refresh and mapping inspection only read information. JSON snapshot files may include process names/PIDs and must be treated as local diagnostic data. Export is only performed when requested; mappings are not included in snapshots. Loaded snapshots are historical and never authorize inspecting their recorded PIDs until live capture is resumed.
"""


def display_bytes(value):
    if value is None:
        return "Unavailable"
    amount = float(value)
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if abs(amount) < 1024 or unit == "TiB":
            return f"{amount:,.1f} {unit}"
        amount /= 1024


class NumberItem(QTableWidgetItem):
    def __init__(self, text, value=None):
        super().__init__(text)
        self.value = value
    def __lt__(self, other):
        a, b = self.value, getattr(other, "value", None)
        if isinstance(a, (int, float)) and isinstance(b, (int, float)):
            return a < b
        return self.text().casefold() < other.text().casefold()


class MemoryWorker(QObject):
    completed = Signal(str, object)
    failed = Signal(str)
    stopped = Signal()

    @Slot(str, object)
    def operate(self, action, payload):
        try:
            from vanta.infra.memory_map import MemoryMapCollector, load_snapshot, save_snapshot
            from pathlib import Path
            if action == "sample":
                result = MemoryMapCollector().sample()
            elif action == "maps":
                result = MemoryMapCollector().memory_maps(*payload)
            elif action == "maintain":
                from vanta.infra.memory_actions import MemoryActions
                result = MemoryActions().execute(**payload)
            elif action == "load":
                result = load_snapshot(Path(payload))
            elif action == "save":
                snapshot, path = payload
                save_snapshot(snapshot, Path(path))
                result = path
            else:
                raise ValueError("Unknown memory operation")
            self.completed.emit(action, result)
        except Exception as exc:
            self.failed.emit(str(exc) or type(exc).__name__)

    @Slot()
    def stop(self):
        self.stopped.emit()


class MemoryMapWidget(QWidget):
    request = Signal(str, object)
    request_stop = Signal()

    def __init__(self, parent=None, *, start_live=True):
        super().__init__(parent)
        self.snapshot = None
        self.busy = self.stopping = self.historical = False
        self.thread = None
        self._mapping_name = ""
        self.mapping_context = ""
        self.dialog_open = False
        self.active_action = None
        self._build_ui()
        self.timer = QTimer(self)
        self.timer.timeout.connect(self._periodic)
        self.timer.start(4000)
        if start_live:
            QTimer.singleShot(0, self.refresh)

    def _build_ui(self):
        layout = QVBoxLayout(self)
        layout.setContentsMargins(20, 18, 20, 20)
        title = QLabel("Memory Map")
        title.setObjectName("sectionTitle")
        layout.addWidget(title)
        self.scope = QLabel("RAM diagnostics + manual maintenance - not full RAMMap parity")
        self.scope.setWordWrap(True)
        layout.addWidget(self.scope)
        bar = QHBoxLayout()
        self.refresh_button = QPushButton("Capture live")
        self.refresh_button.clicked.connect(self.refresh)
        bar.addWidget(self.refresh_button)
        self.rate = QComboBox()
        for label, ms in (("Every 4 seconds", 4000), ("Every 10 seconds", 10000), ("Pause", 0)):
            self.rate.addItem(label, ms)
        self.rate.currentIndexChanged.connect(self._change_rate)
        bar.addWidget(self.rate)
        self.save_button = QPushButton("Save snapshot…")
        self.save_button.setEnabled(False)
        self.save_button.clicked.connect(self._save)
        bar.addWidget(self.save_button)
        self.load_button = QPushButton("Open snapshot…")
        self.load_button.clicked.connect(self._load)
        bar.addWidget(self.load_button)
        bar.addStretch()
        layout.addLayout(bar)
        maintenance = QHBoxLayout()
        self.maintenance_operation = QComboBox()
        for label, operation in (
            ("Trim selected process working set", "trim_process"),
            ("Trim system file-cache working set (admin)", "trim_system_cache"),
            ("Purge standby list (advanced / admin)", "purge_standby"),
            ("Purge priority-0 standby only (advanced / admin)", "purge_standby_priority0"),
        ):
            self.maintenance_operation.addItem(label, operation)
        self.maintenance_operation.currentIndexChanged.connect(self._selection_changed)
        maintenance.addWidget(self.maintenance_operation)
        self.maintenance_button = QPushButton("Review and run...")
        self.maintenance_button.setEnabled(False)
        self.maintenance_button.clicked.connect(self._maintain)
        maintenance.addWidget(self.maintenance_button)
        layout.addLayout(maintenance)
        self.maintenance_result = QLabel("Manual only. Cached RAM is normally reusable; clearing can slow applications.")
        self.maintenance_result.setWordWrap(True)
        self.maintenance_result.setTextFormat(Qt.TextFormat.PlainText)
        layout.addWidget(self.maintenance_result)
        self.status = QLabel("Ready — capture live data to begin.")
        self.status.setWordWrap(True)
        self.status.setTextFormat(Qt.TextFormat.PlainText)
        layout.addWidget(self.status)
        self.tabs = QTabWidget()
        self.overview = self._table(["Metric", "Value", "Source", "Meaning / limitations"])
        self.overview.setSortingEnabled(False)
        self.tabs.addTab(self.overview, "Overview")
        processes_page = QWidget()
        process_layout = QVBoxLayout(processes_page)
        self.search = QLineEdit()
        self.search.setPlaceholderText("Filter process name or PID…")
        self.search.textChanged.connect(self._filter)
        process_layout.addWidget(self.search)
        process_layout.addWidget(QLabel("Working sets can share pages. Private bytes are committed memory, not resident RAM."))
        self.processes = self._table(["Name", "PID", "Working set", "Private bytes", "Status"])
        self.processes.itemSelectionChanged.connect(self._selection_changed)
        process_layout.addWidget(self.processes)
        self.maps_button = QPushButton("Inspect selected process mappings")
        self.maps_button.setEnabled(False)
        self.maps_button.clicked.connect(self._inspect)
        process_layout.addWidget(self.maps_button)
        self.tabs.addTab(processes_page, "Processes")
        self.mappings = self._table(["Path", "Resident bytes", "Mapped size", "Private bytes", "Limitations"])
        self.tabs.addTab(self.mappings, "Process mappings")
        self.capabilities = QPlainTextEdit()
        self.capabilities.setReadOnly(True)
        self.capabilities.setPlainText(CAPABILITIES)
        self.tabs.addTab(self.capabilities, "Scope and definitions")
        layout.addWidget(self.tabs)

    @staticmethod
    def _table(headers):
        table = QTableWidget(0, len(headers))
        table.setHorizontalHeaderLabels(headers)
        table.setAlternatingRowColors(True)
        table.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        table.setSelectionBehavior(QAbstractItemView.SelectionBehavior.SelectRows)
        table.setSelectionMode(QAbstractItemView.SelectionMode.SingleSelection)
        table.verticalHeader().hide()
        table.horizontalHeader().setSectionResizeMode(QHeaderView.ResizeMode.Interactive)
        table.horizontalHeader().setStretchLastSection(True)
        table.setSortingEnabled(True)
        return table

    def _ensure_worker(self):
        if self.thread is not None:
            return
        self.thread = QThread(self)
        self.worker = MemoryWorker()
        self.worker.moveToThread(self.thread)
        self.request.connect(self.worker.operate)
        self.request_stop.connect(self.worker.stop)
        self.worker.completed.connect(self._completed)
        self.worker.failed.connect(self._error)
        self.worker.stopped.connect(self.thread.quit, Qt.ConnectionType.DirectConnection)
        self.thread.finished.connect(self.worker.deleteLater)
        self.thread.start()

    def _operate(self, action, payload=None):
        if self.busy or self.stopping:
            return
        self._ensure_worker()
        self.busy = True
        self.active_action = action
        if action == "maintain":
            self.maintenance_result.setText("Memory maintenance is running. Waiting for Windows result...")
        self.status.setText(f"Reading… ({action})")
        self._controls()
        self.request.emit(action, payload)

    def _controls(self):
        for button in (self.refresh_button, self.load_button):
            button.setEnabled(not self.busy and not self.stopping)
        self.save_button.setEnabled(self.snapshot is not None and not self.busy and not self.stopping)
        self._selection_changed()

    def _change_rate(self):
        ms = self.rate.currentData()
        if ms:
            self.timer.start(ms)
        else:
            self.timer.stop()

    def _periodic(self):
        if self.isVisible() and not self.historical and not self.dialog_open:
            self._operate("sample")

    def refresh(self):
        self._operate("sample")

    def _selected(self):
        rows = self.processes.selectionModel().selectedRows()
        return self.processes.item(rows[0].row(), 0).data(Qt.ItemDataRole.UserRole) if rows else None

    def _selection_changed(self):
        selected = self._selected()
        valid_identity = selected is not None and selected.created > 0
        self.maps_button.setEnabled(not self.busy and not self.stopping and not self.historical and valid_identity)
        if hasattr(self, "maintenance_button"):
            import os
            allowed = (os.name == "nt" and self.snapshot is not None and not self.historical
                       and not self.busy and not self.stopping and not self.dialog_open)
            selected_required = self.maintenance_operation.currentData() == "trim_process"
            self.maintenance_button.setEnabled(allowed and (valid_identity or not selected_required))

    def _inspect(self):
        process = self._selected()
        if process is None or process.created <= 0 or self.historical:
            return
        self.mappings.setRowCount(0)
        self.mapping_context = ""
        self._mapping_name = f"{process.name} (PID {process.pid}, created {process.created})"
        self._operate("maps", (process.pid, process.created))

    def _maintain(self):
        import os
        if (os.name != "nt" or self.snapshot is None or self.historical
                or self.busy or self.stopping or self.dialog_open):
            return
        operation = self.maintenance_operation.currentData()
        payload = {"operation": operation, "confirmed": True}
        target = "system-wide memory state"
        if operation == "trim_process":
            process = self._selected()
            if process is None or process.created <= 0:
                return
            payload.update(pid=process.pid, created=process.created)
            target = f"{process.name} (PID {process.pid})"
        advanced = operation in {"purge_standby", "purge_standby_priority0"}
        text = (f"Operation: {self.maintenance_operation.currentText()}\nTarget: {target}\n\n"
                "This changes live memory state. It does not delete files or fix memory leaks. "
                "Applications may pause, incur page faults, or reload data from disk. "
                "Available RAM is not proof of attributable memory freed.\n\n")
        if advanced:
            text += "ADVANCED: undocumented Windows native operation; compatibility is not guaranteed.\n\n"
        if operation != "trim_process":
            text += "Requires administrator privileges. No automatic elevation will occur.\n\n"
        text += "Run once? Cancel leaves memory unchanged."
        self.dialog_open = True
        self._controls()
        try:
            answer = QMessageBox.warning(self, "Confirm memory maintenance", text,
                QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.Cancel,
                QMessageBox.StandardButton.Cancel)
        finally:
            self.dialog_open = False
            self._controls()
        if answer == QMessageBox.StandardButton.Yes and not self.stopping:
            self._operate("maintain", payload)

    def _save(self):
        snapshot = self.snapshot
        self.dialog_open = True
        try:
            path, _ = QFileDialog.getSaveFileName(self, "Save local diagnostic snapshot", "vanta-memory.json", "VAnta JSON (*.json)")
        finally:
            self.dialog_open = False
        if path and snapshot is not None:
            self._operate("save", (snapshot, path))

    def _load(self):
        self.dialog_open = True
        try:
            path, _ = QFileDialog.getOpenFileName(self, "Open VAnta snapshot (not RAMMap .RMP)", "", "VAnta JSON (*.json)")
        finally:
            self.dialog_open = False
        if path:
            self._operate("load", path)

    @Slot(str, object)
    def _completed(self, action, result):
        self.busy = False
        if self.stopping:
            return
        if action in {"sample", "load"}:
            self.snapshot = result
            self.historical = action == "load"
            if self.historical:
                self.mappings.setRowCount(0)
                self.mapping_context = ""
                self.tabs.setTabToolTip(2, "")
            self._render(result)
        elif action == "maintain":
            from datetime import datetime, timezone
            self.mappings.setRowCount(0)
            self.mapping_context = ""
            self.tabs.setTabToolTip(2, "")
            stamp = datetime.now(timezone.utc).isoformat(timespec="seconds")
            self.maintenance_result.setText(
                f"{stamp} - {result.operation}: {result.message}\n"
                f"Available RAM: {display_bytes(result.before_available)} -> {display_bytes(result.after_available)} "
                "(system observation, not attributable freed bytes).")
            self.status.setText("Maintenance completed. Previous capture is STALE; capturing live data...")
            self._controls()
            self.refresh()
            return
        elif action == "maps":
            from datetime import datetime, timezone
            self.mapping_context = f"{self._mapping_name}, inspected {datetime.now(timezone.utc).isoformat()} (separate capture)"
            self.tabs.setTabToolTip(2, self.mapping_context)
            rows = [(r.path, display_bytes(r.rss), display_bytes(r.size), display_bytes(r.private_bytes), r.note) for r in result]
            raw = [(None, r.rss, r.size, r.private_bytes, None) for r in result]
            self._fill(self.mappings, rows, raw)
            self.tabs.setCurrentWidget(self.mappings)
            self.status.setText(f"{self._mapping_name}: {len(result)} mappings. Not a physical-page inventory.")
        else:
            self.status.setText(f"Snapshot saved: {result}")
        self._controls()

    def _render(self, snapshot):
        self._fill(self.overview, [(m.name, display_bytes(m.value), m.source, m.note) for m in snapshot.metrics],
                   [(None, m.value, None, None) for m in snapshot.metrics])
        rows = [(p.name, str(p.pid), display_bytes(p.working_set), display_bytes(p.private_bytes), p.status) for p in snapshot.processes]
        raw = [(None, p.pid, p.working_set, p.private_bytes, None) for p in snapshot.processes]
        self._fill(self.processes, rows, raw, snapshot.processes)
        self.overview.setColumnWidth(2, 290)
        self._filter()
        mode = "HISTORICAL — auto-refresh and PID inspection disabled" if self.historical else "LIVE"
        warnings = " | ".join(snapshot.warnings[:3])
        if len(snapshot.warnings) > 3:
            warnings += f" | +{len(snapshot.warnings) - 3} further warnings (included in JSON)"
        if self.mapping_context:
            warnings += f" | Mapping capture: {self.mapping_context}"
        self.status.setText(f"{mode} · {snapshot.captured_at} · {len(snapshot.processes)} processes" + (f"\n{warnings}" if warnings else ""))

    @staticmethod
    def _fill(table, rows, raw, objects=None):
        sortable = table.isSortingEnabled()
        table.setSortingEnabled(False)
        table.clearSelection()
        table.setRowCount(len(rows))
        for i, row in enumerate(rows):
            for j, text in enumerate(row):
                item = NumberItem(str(text), raw[i][j])
                item.setToolTip(str(text))
                if j == 0 and objects is not None:
                    item.setData(Qt.ItemDataRole.UserRole, objects[i])
                table.setItem(i, j, item)
        table.setSortingEnabled(sortable)
        for col in range(min(4, table.columnCount())):
            table.setColumnWidth(col, 240 if col == 0 else 140)

    def _filter(self):
        needle = self.search.text().casefold().strip()
        for row in range(self.processes.rowCount()):
            text = " ".join(self.processes.item(row, col).text() for col in (0, 1)).casefold()
            self.processes.setRowHidden(row, needle not in text)

    @Slot(str)
    def _error(self, message):
        maintenance_failed = self.active_action == "maintain"
        if maintenance_failed:
            self.maintenance_result.setText(
                f"Memory maintenance failed: {message}. Memory may have changed before failure; refreshing observations.")
            self.mappings.setRowCount(0)
            self.mapping_context = ""
            self.tabs.setTabToolTip(2, "")
        self.active_action = None
        self.busy = False
        if not self.stopping:
            previous = f" - previous capture {self.snapshot.captured_at} is STALE" if self.snapshot is not None else ""
            self.status.setText(f"Unavailable{previous} - {message}")
            self._controls()
            if maintenance_failed:
                self.refresh()

    def shutdown(self):
        if not self.stopping:
            self.stopping = True
            self.timer.stop()
            if self.thread is not None:
                self.request_stop.emit()
        return self.thread is None or self.thread.wait(100)
