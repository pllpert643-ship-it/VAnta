"""Read-only memory observations, not RAMMap PFN or file-summary data.

All memory quantities are bytes; None means unavailable, never an inferred zero.
Sampling is sequential, not an atomic system-wide capture. No operation changes
process state, privileges, working sets, or caches. Only save_snapshot writes.
"""
from __future__ import annotations

import json
import math
import os
import re
import tempfile
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path

import psutil


@dataclass
class MemoryMetric:
    name: str
    value: int | None
    source: str
    note: str


@dataclass
class MemoryProcess:
    pid: int
    created: float
    name: str
    working_set: int | None
    private_bytes: int | None
    status: str


@dataclass
class MemoryRegion:
    path: str
    rss: int | None
    size: int | None
    private_bytes: int | None
    note: str


@dataclass
class MemorySnapshot:
    captured_at: str
    metrics: list[MemoryMetric]
    processes: list[MemoryProcess]
    warnings: list[str]


# windows_tasks already converts documented GetPerformanceInfo page counts to bytes.
_COUNTERS = (
    ("Committed", "CommitTotal", "Committed bytes; not resident physical usage."),
    ("Commit limit", "CommitLimit", "Commit limit in bytes; may grow with paging files."),
    ("Commit peak", "CommitPeak", "Peak committed bytes since boot."),
    ("System cache", "SystemCache", "Bytes of standby plus system working set; not exact standby."),
    ("Paged kernel pool", "KernelPaged", "Paged kernel pool bytes; not necessarily resident."),
    ("Nonpaged kernel pool", "KernelNonpaged", "Nonpaged kernel pool bytes."),
)


class MemoryMapCollector:
    """Lightweight polling; detailed process mappings are opt-in, never sampled."""

    def sample(self) -> MemorySnapshot:
        warnings = [
            "Accounting domains overlap; do not sum these metrics or process working sets. "
            "Private bytes are committed virtual memory, not private resident RAM.",
            "RAMMap PFN page states, physical ranges and global file summary are unavailable.",
        ]
        total = available = None
        try:
            memory = psutil.virtual_memory()
            total, available = memory.total, memory.available
        except (psutil.Error, OSError, NotImplementedError) as exc:
            warnings.append(f"Physical memory unavailable: {exc}")
        metrics = [
            MemoryMetric("Physical total", total, "psutil.virtual_memory.total", "Physical RAM in bytes."),
            MemoryMetric("Physical available", available, "psutil.virtual_memory.available", "Available physical bytes; not just free memory."),
        ]
        counters = {}
        if os.name == "nt":
            try:
                from vanta.infra.windows_tasks import memory_counters
                counters = memory_counters()
            except (OSError, NotImplementedError) as exc:
                warnings.append(f"GetPerformanceInfo unavailable: {exc}")
        else:
            warnings.append("GetPerformanceInfo unavailable: requires Windows.")
        metrics.extend(MemoryMetric(name, counters.get(name), f"GetPerformanceInfo.{field}", note)
                       for name, field, note in _COUNTERS)
        metrics.extend(MemoryMetric(name, None, "Unavailable",
            "Exact page-state bytes unavailable: no RAMMap PFN query; not inferred from aggregate counters.")
            for name in ("Active", "Standby", "Modified", "Modified no-write", "Zeroed", "Free", "Bad"))
        processes = []
        try:
            for proc in psutil.process_iter():
                row = MemoryProcess(proc.pid, 0.0, f"PID {proc.pid}", None, None, "ok")
                issues = []
                for attribute, getter in (("created", proc.create_time), ("name", proc.name)):
                    try:
                        setattr(row, attribute, getter())
                    except (psutil.Error, OSError) as exc:
                        issues.append(_process_error(exc))
                        if attribute == "created":
                            issues.append("identity unavailable; mappings disabled")
                try:
                    info = proc.memory_info()
                    row.working_set = getattr(info, "rss", None)
                    row.private_bytes = getattr(info, "private", None)
                    if row.private_bytes is None:
                        issues.append("private bytes unavailable")
                except (psutil.Error, OSError) as exc:
                    issues.append(_process_error(exc))
                row.status = "; ".join(dict.fromkeys(issues)) or "ok"
                if issues:
                    warnings.append(f"PID {row.pid}: {row.status}")
                processes.append(row)
        except (psutil.Error, OSError, NotImplementedError) as exc:
            warnings.append(f"Process enumeration unavailable: {exc}")
        return MemorySnapshot(datetime.now(timezone.utc).isoformat(), metrics, processes, warnings)

    def memory_maps(self, pid: int, created: float) -> list[MemoryRegion]:
        """Read mappings grouped by path (not PFN File Summary).

        Raises ValueError for stale identity, PermissionError for restricted
        access, ProcessLookupError for exit, and NotImplementedError when absent.
        No timeout: psutil's native Windows call is synchronous and cannot be
        cancelled safely. Call on a worker; row bounds do not bound call duration.
        Fresh creation-time checks before AND after discard PID-reuse races;
        the mapping read itself is not an atomic handle-bound snapshot.
        """
        _integer(pid, "pid", maximum=2**32 - 1)
        created = _created(created)
        if created == 0:
            raise ValueError("Process identity unavailable; capture live data again.")
        try:
            process = psutil.Process(pid)
            if process.create_time() != created:
                raise ValueError("PID reused or identity changed; capture live data again.")
            regions = process.memory_maps(grouped=True)
            if psutil.Process(pid).create_time() != created:
                raise ValueError("PID reused or identity changed during mapping read.")
        except psutil.AccessDenied as exc:
            raise PermissionError(f"Mappings access denied for PID {pid}.") from exc
        except psutil.NoSuchProcess as exc:
            raise ProcessLookupError(f"PID {pid} exited or disappeared.") from exc
        result = []
        for region in regions:
            if len(result) >= 16384:
                raise ValueError("Too many mappings: 16384 grouped-row limit.")
            note = "Bytes; mappings grouped by path, not RAMMap PFN File Summary."
            rss = getattr(region, "rss", None)
            if os.name == "nt":
                # Windows psutil calls its RegionSize-based field rss. It is not
                # a measured resident working set; never display it as residency
                # or infer an absent size/private field from it.
                note += f" Windows psutil rss={rss!r} is not measured resident bytes; suppressed."
                rss = None
            result.append(MemoryRegion(region.path, rss, getattr(region, "size", None),
                                       getattr(region, "private", None), note))
        return result


def _process_error(exc: Exception) -> str:
    if isinstance(exc, psutil.AccessDenied):
        return "access denied"
    if isinstance(exc, psutil.NoSuchProcess):
        return "process exited or disappeared"
    return f"unavailable: {exc}"


SCHEMA_VERSION = 1
MAX_SNAPSHOT_BYTES = 16 * 1024 * 1024
_UNITS = {"memory": "bytes", "created": "unix_seconds", "captured_at": "UTC"}


def _object(value, keys: set[str], label: str) -> dict:
    if type(value) is not dict or set(value) != keys:
        raise ValueError(f"Invalid {label} fields.")
    return value


def _text(value, label: str) -> str:
    if type(value) is not str or len(value) > 32768:
        raise ValueError(f"Invalid {label}: expected bounded text.")
    return value


def _integer(value, label: str, *, nullable: bool = False, maximum: int = 2**64 - 1):
    if value is None and nullable:
        return None
    if type(value) is not int or not 0 <= value <= maximum:
        raise ValueError(f"Invalid {label}: expected nonnegative integer.")
    return value


def _created(value) -> float:
    if type(value) not in (int, float) or not 0 <= value <= 253402300799 or not math.isfinite(value):
        raise ValueError("Invalid created: expected finite nonnegative Unix seconds.")
    return float(value)


def _list(value, limit: int, label: str) -> list:
    if type(value) is not list or len(value) > limit:
        raise ValueError(f"Invalid {label}: expected list of at most {limit} entries.")
    return value


def _snapshot_from_dict(data) -> MemorySnapshot:
    _object(data, {"schema_version", "units", "captured_at", "metrics", "processes", "warnings"}, "snapshot")
    if type(data["schema_version"]) is not int or data["schema_version"] != SCHEMA_VERSION:
        raise ValueError("Unsupported snapshot schema_version.")
    if type(data["units"]) is not dict or data["units"] != _UNITS:
        raise ValueError("Invalid snapshot units.")
    captured = _text(data["captured_at"], "captured_at")
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?(?:Z|\+00:00)", captured):
        raise ValueError("captured_at must be an ISO-8601 UTC timestamp.")
    datetime.fromisoformat(captured)
    metrics = []
    for item in _list(data["metrics"], 256, "metrics"):
        _object(item, {"name", "value", "source", "note"}, "metric")
        metrics.append(MemoryMetric(_text(item["name"], "name"),
            _integer(item["value"], "value", nullable=True),
            _text(item["source"], "source"), _text(item["note"], "note")))
    processes = []
    for item in _list(data["processes"], 65536, "processes"):
        _object(item, {"pid", "created", "name", "working_set", "private_bytes", "status"}, "process")
        processes.append(MemoryProcess(_integer(item["pid"], "pid", maximum=2**32 - 1),
            _created(item["created"]), _text(item["name"], "name"),
            _integer(item["working_set"], "working_set", nullable=True),
            _integer(item["private_bytes"], "private_bytes", nullable=True), _text(item["status"], "status")))
    warnings = [_text(item, "warning") for item in _list(data["warnings"], 65536, "warnings")]
    return MemorySnapshot(captured, metrics, processes, warnings)


def snapshot_to_dict(snapshot: MemorySnapshot) -> dict:
    """Return independent validated JSON data with explicit byte/time units."""
    if not isinstance(snapshot, MemorySnapshot):
        raise ValueError("Expected MemorySnapshot.")
    data = {"schema_version": SCHEMA_VERSION, "units": dict(_UNITS), **asdict(snapshot)}
    _snapshot_from_dict(data)
    return data


def save_snapshot(snapshot: MemorySnapshot, path: Path) -> None:
    """Validate, sync a same-directory temporary export, then replace atomically."""
    encoded = json.dumps(snapshot_to_dict(snapshot), ensure_ascii=True, allow_nan=False).encode("utf-8")
    if len(encoded) > MAX_SNAPSHOT_BYTES:
        raise ValueError("Snapshot size exceeds 16 MiB limit.")
    temporary_path = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="wb", dir=path.parent, prefix=f".{path.name}.", suffix=".tmp", delete=False
        ) as stream:
            temporary_path = Path(stream.name)
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary_path, path)
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)


def load_snapshot(path: Path) -> MemorySnapshot:
    """Import strict local VAnta JSON, not RAMMap .RMP; bad data raises ValueError."""
    with path.open("rb") as stream:
        raw = stream.read(MAX_SNAPSHOT_BYTES + 1)
    if len(raw) > MAX_SNAPSHOT_BYTES:
        raise ValueError("Snapshot size exceeds 16 MiB limit.")
    try:
        return _snapshot_from_dict(json.loads(raw.decode("utf-8"),
            object_pairs_hook=_unique_object, parse_constant=_invalid_constant))
    except (UnicodeError, RecursionError) as exc:
        raise ValueError("Malformed snapshot encoding or nesting.") from exc


def _unique_object(pairs) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate snapshot field: {key}")
        result[key] = value
    return result


def _invalid_constant(value):
    raise ValueError(f"Invalid JSON numeric constant: {value}")
