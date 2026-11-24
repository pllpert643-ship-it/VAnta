"""Behavior checks for the read-only Memory Map workspace."""
import os
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
from PySide6.QtWidgets import QApplication


def test_workspace_explains_manual_maintenance_and_missing_rammap_features():
    from vanta.ui.memory_map import MemoryMapWidget
    app = QApplication.instance() or QApplication([])
    widget = MemoryMapWidget(start_live=False)
    try:
        assert widget.tabs.count() == 4
        assert "manual maintenance" in widget.scope.text().lower()
        assert "undocumented" in widget.capabilities.toPlainText()
        assert "PFN" in widget.capabilities.toPlainText()
        assert not widget.maps_button.isEnabled()
    finally:
        assert widget.shutdown()
        widget.deleteLater()
        app.processEvents()


def test_historical_snapshot_cannot_inspect_live_pid_and_capture_resumes():
    from types import SimpleNamespace
    from vanta.ui.memory_map import MemoryMapWidget
    app = QApplication.instance() or QApplication([])
    widget = MemoryMapWidget(start_live=False)
    process = SimpleNamespace(name="example", pid=1234, created=10.0,
                              working_set=1024, private_bytes=None, status="readable")
    snapshot = SimpleNamespace(captured_at="2026-11-16T00:00:00+00:00",
        metrics=[SimpleNamespace(name="Physical total", value=4096, source="test", note="physical")],
        processes=[process], warnings=[])
    try:
        widget._completed("load", snapshot)
        widget.processes.selectRow(0)
        assert widget.historical
        assert not widget.maps_button.isEnabled()
        assert "HISTORICAL" in widget.status.text()
        assert widget.processes.item(0, 3).text() == "Unavailable"
        widget._completed("sample", snapshot)
        widget.processes.selectRow(0)
        assert not widget.historical
        assert widget.maps_button.isEnabled()
        widget.search.setText("missing")
        assert widget.processes.isRowHidden(0)
    finally:
        assert widget.shutdown()
        widget.deleteLater()
        app.processEvents()


def test_numeric_columns_sort_as_numbers_not_formatted_strings():
    from vanta.ui.memory_map import MemoryMapWidget
    app = QApplication.instance() or QApplication([])
    widget = MemoryMapWidget(start_live=False)
    try:
        widget._fill(widget.processes,
            [("big", "20", "1.0 MiB", "Unavailable", "ok"),
             ("small", "3", "900.0 KiB", "Unavailable", "ok")],
            [(None, 20, 1048576, None, None), (None, 3, 921600, None, None)])
        from PySide6.QtCore import Qt
        widget.processes.sortItems(2, Qt.SortOrder.AscendingOrder)
        assert widget.processes.item(0, 0).text() == "small"
    finally:
        assert widget.shutdown()
        widget.deleteLater()
        app.processEvents()


def test_missing_creation_time_disables_mapping_inspection():
    from types import SimpleNamespace
    from vanta.ui.memory_map import MemoryMapWidget
    app = QApplication.instance() or QApplication([])
    widget = MemoryMapWidget(start_live=False)
    p = SimpleNamespace(name="restricted", pid=1, created=0,
        working_set=None, private_bytes=None, status="identity unavailable")
    snapshot = SimpleNamespace(captured_at="2026-11-16T00:00:00+00:00",
                               metrics=[], processes=[p], warnings=[])
    try:
        widget._completed("sample", snapshot)
        widget.processes.selectRow(0)
        assert not widget.maps_button.isEnabled()
        widget._error("access denied")
        assert "STALE" in widget.status.text()
        assert widget.snapshot is snapshot
    finally:
        assert widget.shutdown()
        widget.deleteLater()
        app.processEvents()


def test_live_worker_returns_real_memory_and_stops_cleanly():
    import time
    from vanta.ui.memory_map import MemoryMapWidget
    app = QApplication.instance() or QApplication([])
    widget = MemoryMapWidget(start_live=False)
    try:
        widget.refresh()
        deadline = time.monotonic() + 20
        while widget.busy and time.monotonic() < deadline:
            app.processEvents()
            time.sleep(0.01)
        assert not widget.busy, "Memory worker did not return within test deadline"
        assert widget.snapshot is not None, widget.status.text()
        assert widget.processes.rowCount() > 0
        assert any(p.pid == os.getpid() for p in widget.snapshot.processes)
        assert widget.overview.rowCount() >= 2
    finally:
        deadline = time.monotonic() + 10
        while not widget.shutdown() and time.monotonic() < deadline:
            app.processEvents()
            time.sleep(0.01)
        assert widget.thread is None or not widget.thread.isRunning()
        widget.deleteLater()
        app.processEvents()


def test_mapping_capture_survives_ordinary_refresh(monkeypatch):
    from types import SimpleNamespace
    from vanta.ui.memory_map import MemoryMapWidget
    app = QApplication.instance() or QApplication([])
    widget = MemoryMapWidget(start_live=False)
    snapshot = SimpleNamespace(captured_at="2026-11-16T00:00:00+00:00", metrics=[], processes=[], warnings=[])
    try:
        widget._mapping_name = "example PID 123"
        widget._completed("maps", [SimpleNamespace(path="example.dll", rss=None, size=None, private_bytes=None, note="Not PFN")])
        widget._completed("sample", snapshot)
        assert widget.mappings.rowCount() == 1
        assert "example PID 123" in widget.tabs.tabToolTip(2)
        widget._completed("load", snapshot)
        assert widget.mappings.rowCount() == 0
    finally:
        assert widget.shutdown()
        widget.deleteLater()
        app.processEvents()


def test_file_dialog_pauses_automatic_capture(monkeypatch, tmp_path):
    from types import SimpleNamespace
    from vanta.ui.memory_map import MemoryMapWidget, QFileDialog
    app = QApplication.instance() or QApplication([])
    widget = MemoryMapWidget(start_live=False)
    calls = []
    snapshot = SimpleNamespace(captured_at="original")
    widget.snapshot = snapshot
    monkeypatch.setattr(widget, "_operate", lambda action, payload=None: calls.append((action, payload)))
    monkeypatch.setattr(widget, "isVisible", lambda: True)
    def choose(*args):
        widget._periodic()
        widget.snapshot = SimpleNamespace(captured_at="new")
        return str(tmp_path / "test.json"), ""
    monkeypatch.setattr(QFileDialog, "getSaveFileName", choose)
    try:
        widget._save()
        assert calls == [("save", (snapshot, str(tmp_path / "test.json")))]
        assert not widget.dialog_open
    finally:
        assert widget.shutdown()
        widget.deleteLater()
        app.processEvents()


def test_memory_maintenance_cancel_never_dispatches(monkeypatch):
    from types import SimpleNamespace
    from vanta.ui.memory_map import MemoryMapWidget, QMessageBox
    app = QApplication.instance() or QApplication([])
    widget = MemoryMapWidget(start_live=False)
    calls = []
    try:
        widget._completed("sample", SimpleNamespace(captured_at="now", metrics=[], processes=[], warnings=[]))
        widget.maintenance_operation.setCurrentIndex(1)
        monkeypatch.setattr(QMessageBox, "warning", lambda *a, **k: QMessageBox.StandardButton.Cancel)
        monkeypatch.setattr(widget, "_operate", lambda *a: calls.append(a))
        widget.maintenance_button.click()
        assert calls == []
        assert not widget.dialog_open
    finally:
        assert widget.shutdown()
        widget.deleteLater()
        app.processEvents()


def test_maintenance_confirmation_freezes_refresh_and_selected_identity(monkeypatch):
    from types import SimpleNamespace
    from vanta.ui.memory_map import MemoryMapWidget, QMessageBox
    app = QApplication.instance() or QApplication([])
    widget = MemoryMapWidget(start_live=False)
    calls = []
    process = SimpleNamespace(name="owned-test", pid=12345, created=20.0,
                              working_set=1024, private_bytes=2048, status="readable")
    snapshot = SimpleNamespace(captured_at="now", metrics=[], processes=[process], warnings=[])
    try:
        widget._completed("sample", snapshot)
        widget.processes.selectRow(0)
        monkeypatch.setattr(widget, "isVisible", lambda: True)
        monkeypatch.setattr(widget, "_operate", lambda *a: calls.append(a))
        def confirm(*args, **kwargs):
            assert widget.dialog_open
            assert not widget.maintenance_button.isEnabled()
            widget._periodic()
            return QMessageBox.StandardButton.Yes
        monkeypatch.setattr(QMessageBox, "warning", confirm)
        widget.maintenance_button.click()
        assert calls == [("maintain", {"operation": "trim_process", "confirmed": True,
                                      "pid": 12345, "created": 20.0})]
        widget._completed("load", snapshot)
        widget._maintain()
        assert len(calls) == 1
        assert not widget.maintenance_button.isEnabled()
    finally:
        assert widget.shutdown()
        widget.deleteLater()
        app.processEvents()


def test_maintenance_result_persists_through_live_refresh(monkeypatch):
    from types import SimpleNamespace
    from vanta.ui.memory_map import MemoryMapWidget
    app = QApplication.instance() or QApplication([])
    widget = MemoryMapWidget(start_live=False)
    refreshes = []
    monkeypatch.setattr(widget, "refresh", lambda: refreshes.append(True))
    try:
        widget._completed("maintain", SimpleNamespace(operation="trim_process", message="Completed",
                          before_available=1024, after_available=2048))
        assert refreshes == [True]
        assert "not attributable freed bytes" in widget.maintenance_result.text()
        widget._completed("sample", SimpleNamespace(captured_at="now", metrics=[], processes=[], warnings=[]))
        assert "trim_process" in widget.maintenance_result.text()
    finally:
        assert widget.shutdown()
        widget.deleteLater()
        app.processEvents()


def test_maintenance_failure_is_not_hidden_by_automatic_refresh(monkeypatch):
    from types import SimpleNamespace
    from vanta.ui.memory_map import MemoryMapWidget
    app = QApplication.instance() or QApplication([])
    widget = MemoryMapWidget(start_live=False)
    refreshes = []
    monkeypatch.setattr(widget, "refresh", lambda: refreshes.append(True))
    try:
        widget._mapping_name = "pre-maintenance"
        widget._completed("maps", [SimpleNamespace(path="old.dll", rss=None, size=None, private_bytes=None, note="old")])
        widget.active_action = "maintain"
        widget._error("access denied")
        assert refreshes == [True]
        assert widget.mappings.rowCount() == 0
        assert widget.mapping_context == ""
        assert "may have changed" in widget.maintenance_result.text()
        widget._completed("sample", SimpleNamespace(captured_at="now", metrics=[], processes=[], warnings=[]))
        assert "failed" in widget.maintenance_result.text()
        assert "access denied" in widget.maintenance_result.text()
    finally:
        assert widget.shutdown()
        widget.deleteLater()
        app.processEvents()
