"""Read-only memory observation and snapshot contract tests."""
import json
from types import SimpleNamespace

import psutil
import pytest

from vanta.infra import memory_map
from vanta.infra import windows_tasks


def test_global_metrics_are_bytes_and_cache_is_not_exact_standby(monkeypatch):
    monkeypatch.setattr(memory_map.psutil, "virtual_memory", lambda: SimpleNamespace(total=8192, available=3072))
    monkeypatch.setattr(memory_map.psutil, "process_iter", lambda: iter(()))
    monkeypatch.setattr(windows_tasks, "memory_counters", lambda: {
        "Committed": 4096, "Commit limit": 16384, "Commit peak": 6144,
        "System cache": 2048, "Paged kernel pool": 512, "Nonpaged kernel pool": 256,
    })
    snapshot = memory_map.MemoryMapCollector().sample()
    metrics = {metric.name: metric for metric in snapshot.metrics}
    assert metrics["Physical total"].value == 8192
    assert metrics["Physical available"].value == 3072
    assert metrics["System cache"].value == 2048
    assert metrics["System cache"].source == "GetPerformanceInfo.SystemCache"
    assert "standby" in metrics["System cache"].note.lower()
    assert "system working set" in metrics["System cache"].note.lower()
    assert "not exact standby" in metrics["System cache"].note.lower()
    assert all("bytes" in metric.note.lower() for metric in snapshot.metrics)
    assert metrics["Standby"].value is None
    assert snapshot.captured_at.endswith("+00:00")
    assert snapshot.processes == []


def test_missing_global_counters_remain_unknown_with_warnings(monkeypatch):
    def unavailable():
        raise OSError("restricted")
    monkeypatch.setattr(memory_map.psutil, "virtual_memory", unavailable)
    monkeypatch.setattr(windows_tasks, "memory_counters", unavailable)
    monkeypatch.setattr(memory_map.psutil, "process_iter", lambda: iter(()))
    snapshot = memory_map.MemoryMapCollector().sample()
    assert len(snapshot.metrics) == 15
    assert all(metric.value is None for metric in snapshot.metrics)
    assert any("physical" in warning.lower() for warning in snapshot.warnings)
    assert any("GetPerformanceInfo" in warning for warning in snapshot.warnings)

class ObservedProcess:
    def __init__(self, pid, *, info=None, denied=False, gone=False, no_identity=False):
        self.pid = pid
        self.info = info or SimpleNamespace(rss=2048, private=1024)
        self.denied, self.gone, self.no_identity = denied, gone, no_identity

    def create_time(self):
        if self.no_identity:
            raise psutil.AccessDenied(self.pid)
        return 100.5

    def name(self):
        return "example.exe"

    def memory_info(self):
        if self.gone:
            raise psutil.NoSuchProcess(self.pid)
        if self.denied:
            raise psutil.AccessDenied(self.pid)
        return self.info


def test_process_sample_keeps_restricted_rows_and_never_substitutes_vms(monkeypatch):
    monkeypatch.setattr(memory_map.psutil, "process_iter", lambda: iter([
        ObservedProcess(10), ObservedProcess(11, denied=True),
        ObservedProcess(12, info=SimpleNamespace(rss=0, vms=999)),
        ObservedProcess(13, gone=True), ObservedProcess(14, no_identity=True),
    ]))
    snapshot = memory_map.MemoryMapCollector().sample()
    rows = {row.pid: row for row in snapshot.processes}
    assert rows[10] == memory_map.MemoryProcess(10, 100.5, "example.exe", 2048, 1024, "ok")
    assert rows[11].working_set is None and rows[11].private_bytes is None
    assert "access denied" in rows[11].status.lower()
    assert rows[12].working_set == 0 and rows[12].private_bytes is None
    assert "private" in rows[12].status.lower()
    assert "exited" in rows[13].status.lower()
    assert rows[14].created == 0.0
    assert "identity unavailable" in rows[14].status.lower()
    assert snapshot.warnings

class MappingProcess:
    def __init__(self, created=100.5, *, error=None):
        self.created, self.error, self.map_calls = created, error, 0

    def create_time(self):
        return self.created

    def memory_maps(self, grouped=True):
        self.map_calls += 1
        assert grouped is True
        if self.error:
            raise self.error
        return [SimpleNamespace(path="example.dll", rss=0)]


def test_on_demand_windows_maps_do_not_invent_size_private_or_residency(monkeypatch):
    process = MappingProcess()
    monkeypatch.setattr(memory_map.psutil, "Process", lambda pid: process)
    regions = memory_map.MemoryMapCollector().memory_maps(42, 100.5)
    assert process.map_calls == 1
    assert len(regions) == 1
    region = regions[0]
    assert region.path == "example.dll"
    assert region.rss is None and region.size is None and region.private_bytes is None
    assert "not" in region.note.lower() and "resident" in region.note.lower()
    assert "PFN" in region.note
    assert "0" in region.note

@pytest.mark.parametrize("changed_after", [False, True])
def test_pid_reuse_discards_mapping_results(monkeypatch, changed_after):
    original = MappingProcess(created=100.5 if changed_after else 200.0)
    fresh = iter([original, MappingProcess(created=200.0)])
    monkeypatch.setattr(memory_map.psutil, "Process", lambda pid: next(fresh))
    with pytest.raises(ValueError, match="reused|changed"):
        memory_map.MemoryMapCollector().memory_maps(42, 100.5)
    assert original.map_calls == int(changed_after)


def example_snapshot():
    return memory_map.MemorySnapshot("2026-11-16T08:00:00+00:00",
        [memory_map.MemoryMetric("Physical total", 8192, "test", "Bytes.")],
        [memory_map.MemoryProcess(42, 100.5, "example.exe", 2048, None, "ok")], ["Not atomic."])


def test_snapshot_round_trip_has_version_and_explicit_units(tmp_path):
    snapshot = example_snapshot()
    payload = memory_map.snapshot_to_dict(snapshot)
    assert payload["schema_version"] == 1
    assert payload["units"] == {"memory": "bytes", "created": "unix_seconds", "captured_at": "UTC"}
    path = tmp_path / "memory.json"
    memory_map.save_snapshot(snapshot, path)
    assert json.loads(path.read_text(encoding="utf-8")) == payload
    assert memory_map.load_snapshot(path) == snapshot
    payload["processes"][0]["name"] = "changed"
    assert snapshot.processes[0].name == "example.exe"

@pytest.mark.parametrize("field,value", [
    ("metric", -1), ("metric", True), ("metric", 1.5), ("metric", float("nan")),
    ("metric", float("inf")), ("metric", "12"), ("metric", 2**64),
    ("pid", -1), ("pid", False), ("pid", 1.5),
    ("created", -1), ("created", True), ("created", float("nan")), ("created", float("inf")),
    ("working_set", -1), ("private_bytes", False),
    ("schema_version", 2), ("schema_version", True),
    ("captured_at", "yesterday"), ("captured_at", "2026-11-16T08:00:00"),
    ("captured_at", "2026-11-16T08:00:00+07:00"),
    ("units", {"memory": "pages"}), ("warnings", "wrong"), ("warnings", [42]),
    ("metrics", {}), ("processes", [None]), ("extra", "unknown"),
])
def test_load_rejects_invalid_schema_and_numbers(tmp_path, field, value):
    payload = memory_map.snapshot_to_dict(example_snapshot())
    if field == "metric":
        payload["metrics"][0]["value"] = value
    elif field in {"pid", "created", "working_set", "private_bytes"}:
        payload["processes"][0][field] = value
    else:
        payload[field] = value
    path = tmp_path / "invalid.json"
    path.write_text(json.dumps(payload), encoding="utf-8")
    with pytest.raises(ValueError):
        memory_map.load_snapshot(path)

@pytest.mark.parametrize("raw", [
    b'{}', b'[]', b'null', b'not json', b'\xff',
    b'{"schema_version":1,"schema_version":1}',
    b'{"schema_version":NaN}', b'[' * 1100 + b']' * 1100,
])
def test_malformed_json_is_rejected_as_value_error(tmp_path, raw):
    path = tmp_path / "malformed.json"
    path.write_bytes(raw)
    with pytest.raises(ValueError):
        memory_map.load_snapshot(path)


def test_duplicate_nested_keys_are_rejected(tmp_path):
    raw = json.dumps(memory_map.snapshot_to_dict(example_snapshot()))
    raw = raw.replace('"pid": 42', '"pid": 42, "pid": 43')
    path = tmp_path / "duplicate.json"
    path.write_text(raw, encoding="utf-8")
    with pytest.raises(ValueError, match="[Dd]uplicate"):
        memory_map.load_snapshot(path)


def test_oversized_snapshot_read_and_export_are_bounded(tmp_path):
    limit = 16 * 1024 * 1024
    path = tmp_path / "huge.json"
    with path.open("wb") as stream:
        stream.write(b' ' * (limit + 1))
    with pytest.raises(ValueError, match="[Ll]arge|[Ss]ize|[Ll]imit"):
        memory_map.load_snapshot(path)
    snapshot = example_snapshot()
    snapshot.warnings = ["x" * 32768] * 513
    path.write_bytes(b"preserve me")
    with pytest.raises(ValueError, match="[Ll]arge|[Ss]ize|[Ll]imit"):
        memory_map.save_snapshot(snapshot, path)
    assert path.read_bytes() == b"preserve me"

@pytest.mark.parametrize("error,expected", [
    (psutil.AccessDenied(42), PermissionError),
    (psutil.NoSuchProcess(42), ProcessLookupError),
    (NotImplementedError("not supported"), NotImplementedError),
])
def test_mapping_errors_are_explicit(monkeypatch, error, expected):
    process = MappingProcess(error=error)
    monkeypatch.setattr(memory_map.psutil, "Process", lambda pid: process)
    with pytest.raises(expected):
        memory_map.MemoryMapCollector().memory_maps(42, 100.5)


@pytest.mark.parametrize("pid,created", [(True, 100.5), (-1, 100.5), (42, 0.0),
    (42, float("nan")), (42, float("inf")), (42, True)])
def test_invalid_identity_never_queries_process(monkeypatch, pid, created):
    def forbidden(*args):
        pytest.fail("invalid identity reached process query")
    monkeypatch.setattr(memory_map.psutil, "Process", forbidden)
    with pytest.raises(ValueError):
        memory_map.MemoryMapCollector().memory_maps(pid, created)


def test_page_state_rows_are_explicitly_unavailable(monkeypatch):
    monkeypatch.setattr(memory_map.psutil, "process_iter", lambda: iter(()))
    snapshot = memory_map.MemoryMapCollector().sample()
    metrics = {metric.name: metric for metric in snapshot.metrics}
    for name in ("Active", "Standby", "Modified", "Modified no-write", "Zeroed", "Free", "Bad"):
        assert metrics[name].value is None
        assert metrics[name].source == "Unavailable"
        assert "PFN" in metrics[name].note
        assert "bytes" in metrics[name].note.lower()
    assert any("overlap" in warning.lower() for warning in snapshot.warnings)


def test_mapping_output_is_bounded_without_claiming_native_timeout(monkeypatch):
    process = MappingProcess()
    def many_maps(grouped=True):
        return [SimpleNamespace(path="many.dll", rss=0)] * 16385
    process.memory_maps = many_maps
    monkeypatch.setattr(memory_map.psutil, "Process", lambda pid: process)
    with pytest.raises(ValueError, match="[Ll]imit|[Mm]any"):
        memory_map.MemoryMapCollector().memory_maps(42, 100.5)


def test_current_process_live_sample_and_on_demand_maps_round_trip(tmp_path):
    import os
    collector = memory_map.MemoryMapCollector()
    snapshot = collector.sample()
    process = next(row for row in snapshot.processes if row.pid == os.getpid())
    assert process.created > 0
    assert process.working_set > 0
    assert next(m for m in snapshot.metrics if m.name == "Physical total").value > 0
    regions = collector.memory_maps(process.pid, process.created)
    assert regions
    assert all(isinstance(region, memory_map.MemoryRegion) for region in regions)
    if os.name == "nt":
        assert all(region.rss is None and region.size is None and region.private_bytes is None for region in regions)
    path = tmp_path / "live.json"
    memory_map.save_snapshot(snapshot, path)
    assert memory_map.load_snapshot(path) == snapshot
    with pytest.raises(ValueError, match="reused|changed"):
        collector.memory_maps(process.pid, process.created - 10)


def test_snapshot_sync_failure_preserves_existing_destination(tmp_path, monkeypatch):
    path = tmp_path / "memory.json"
    path.write_bytes(b"preserve me")

    def fail_sync(fd):
        raise OSError("sync failed")

    monkeypatch.setattr(memory_map.os, "fsync", fail_sync)
    with pytest.raises(OSError, match="sync failed"):
        memory_map.save_snapshot(example_snapshot(), path)

    assert path.read_bytes() == b"preserve me"
    assert set(tmp_path.iterdir()) == {path}


@pytest.mark.parametrize("existing", [False, True])
def test_snapshot_replace_failure_preserves_destination_and_removes_temp(tmp_path, monkeypatch, existing):
    path = tmp_path / "memory.json"
    if existing:
        path.write_bytes(b"preserve me")
    attempted = []

    def fail_replace(source, destination):
        assert source.parent == path.parent
        assert source != path
        assert destination == path
        assert memory_map.load_snapshot(source) == example_snapshot()
        attempted.append(source)
        raise PermissionError("replacement denied")

    monkeypatch.setattr(memory_map.os, "replace", fail_replace)
    with pytest.raises(PermissionError, match="replacement denied"):
        memory_map.save_snapshot(example_snapshot(), path)

    assert len(attempted) == 1
    assert not attempted[0].exists()
    if existing:
        assert path.read_bytes() == b"preserve me"
    else:
        assert not path.exists()
    assert set(tmp_path.iterdir()) == ({path} if existing else set())


@pytest.mark.parametrize("failure_stage", ["write", "flush"])
def test_snapshot_export_failure_preserves_original_and_removes_partial_temp(tmp_path, monkeypatch, failure_stage):
    path = tmp_path / "memory.json"
    path.write_bytes(b"preserve me")
    temporary_file = memory_map.tempfile.NamedTemporaryFile

    class FailingExport:
        def __init__(self, *args, **kwargs):
            self.stream = temporary_file(*args, **kwargs)
            self.name = self.stream.name

        def __enter__(self):
            return self

        def __exit__(self, *args):
            self.stream.close()

        def write(self, data):
            if failure_stage == "write":
                self.stream.write(data[:10])
                self.stream.flush()
                raise OSError("partial write failed")
            return self.stream.write(data)

        def flush(self):
            raise OSError("flush failed")

    monkeypatch.setattr(memory_map.tempfile, "NamedTemporaryFile", FailingExport)
    with pytest.raises(OSError, match="write failed|flush failed"):
        memory_map.save_snapshot(example_snapshot(), path)

    assert path.read_bytes() == b"preserve me"
    assert set(tmp_path.iterdir()) == {path}


@pytest.mark.parametrize("existing", [False, True])
def test_snapshot_success_syncs_complete_export_before_atomic_round_trip(tmp_path, monkeypatch, existing):
    path = tmp_path / "memory.json"
    if existing:
        path.write_bytes(b"old snapshot")
    snapshot = example_snapshot()
    snapshot.warnings.append("Unicode: \u03bb")
    payload = memory_map.snapshot_to_dict(snapshot)
    encoded = json.dumps(payload, ensure_ascii=True, allow_nan=False).encode("utf-8")
    fsync = memory_map.os.fsync
    replace = memory_map.os.replace
    events = []

    def record_sync(fd):
        temporary_paths = set(tmp_path.iterdir()) - {path}
        assert len(temporary_paths) == 1
        temporary_path = temporary_paths.pop()
        assert temporary_path.read_bytes() == encoded
        if existing:
            assert path.read_bytes() == b"old snapshot"
        else:
            assert not path.exists()
        fsync(fd)
        events.append("synced")

    def record_replace(source, destination):
        assert events == ["synced"]
        assert source.parent == path.parent
        assert source != path
        assert destination == path
        replace(source, destination)
        events.append("replaced")

    monkeypatch.setattr(memory_map.os, "fsync", record_sync)
    monkeypatch.setattr(memory_map.os, "replace", record_replace)
    memory_map.save_snapshot(snapshot, path)

    assert events == ["synced", "replaced"]
    assert path.read_bytes() == encoded
    assert memory_map.load_snapshot(path) == snapshot
    assert set(tmp_path.iterdir()) == {path}
