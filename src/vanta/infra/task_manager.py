"""System observation and process control behind a PID + creation-time interface."""
from __future__ import annotations

import os
import re
import sqlite3
import subprocess
import time
from dataclasses import dataclass, field
from pathlib import Path

import psutil


@dataclass(frozen=True)
class ProcessIdentity:
    pid: int
    created: float


@dataclass
class ProcessRow:
    pid: int
    created: float
    name: str
    user: str = ""
    status: str = ""
    cpu: float = 0
    memory: int = 0
    private: int = 0
    threads: int = 0
    handles: int = 0
    cpu_time: float = 0
    read_bytes: int = 0
    write_bytes: int = 0
    read_rate: float = 0
    write_rate: float = 0
    parent: int = 0
    session: int | None = None
    executable: str = ""
    command: str = ""
    priority: str = ""
    accessible: bool = True
    gpu: float | None = None
    gpu_engine: str = ""
    gpu_dedicated: int | None = None
    gpu_shared: int | None = None

    @property
    def identity(self) -> ProcessIdentity:
        return ProcessIdentity(self.pid, self.created)


@dataclass
class SystemSnapshot:
    timestamp: float
    cpu_percent: float
    cpu_per_core: list[float]
    cpu_kernel: float
    memory_total: int
    memory_used: int
    memory_available: int
    memory_percent: float
    uptime: float
    processes: list[ProcessRow]
    memory_details: dict = field(default_factory=dict)
    cpu_details: dict = field(default_factory=dict)
    disks: list[dict] = field(default_factory=list)
    networks: list[dict] = field(default_factory=list)
    gpus: list[dict] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    hardware: dict = field(default_factory=dict)
    gpu_engines: list[dict] = field(default_factory=list)


class TaskManager:
    """Owns sampling baselines and local history; instantiate on its worker thread."""

    def __init__(self, history_path: Path):
        history_path.parent.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(history_path)
        self.db.execute("CREATE TABLE IF NOT EXISTS app_usage (application TEXT PRIMARY KEY, cpu_seconds REAL NOT NULL, read_bytes INTEGER NOT NULL, write_bytes INTEGER NOT NULL, last_seen REAL NOT NULL)")
        self.db.execute("CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value REAL)")
        self.db.execute("INSERT OR IGNORE INTO metadata VALUES ('since', ?)", (time.time(),))
        self.db.commit()
        self._processes: dict[int, psutil.Process] = {}
        self._previous: dict[ProcessIdentity, tuple[float, int, int]] = {}
        self._last_sample = time.monotonic()
        self._disks = psutil.disk_io_counters(perdisk=True) or {}
        self._network = psutil.net_io_counters(pernic=True) or {}
        self._nvml = None
        self._gpu_error = ""
        try:
            import pynvml
            pynvml.nvmlInit()
            self._nvml = pynvml
        except Exception as exc:
            self._gpu_error = str(exc)
        self.hardware = {}
        self._gpu_counters = None
        self._hardware_warning = ""
        if os.name == "nt":
            from vanta.infra.windows_tasks import hardware_inventory, GpuCounters
            try:
                self.hardware = hardware_inventory()
            except Exception as exc:
                self._hardware_warning = str(exc)
            try:
                self._gpu_counters = GpuCounters()
            except OSError:
                pass
        psutil.cpu_percent(percpu=True)
        psutil.cpu_times_percent()

    def close(self):
        self.db.close()
        if self._gpu_counters:
            self._gpu_counters.close()
        if self._nvml:
            self._nvml.nvmlShutdown()

    def sample(self) -> SystemSnapshot:
        now = time.monotonic()
        elapsed = max(now - self._last_sample, .001)
        rows = []
        baseline = {}
        alive = set()
        cpu_count = psutil.cpu_count() or 1
        for discovered in psutil.process_iter():
            pid = discovered.pid
            alive.add(pid)
            proc = self._processes.setdefault(pid, discovered)
            try:
                created = proc.create_time()
                # A cached psutil.Process can outlive a PID: replace it on reuse.
                if not proc.is_running():
                    self._processes.pop(pid, None)
                    continue
                row = ProcessRow(pid, created, proc.name())
                with proc.oneshot():
                    for attribute, getter in (
                        ("user", proc.username), ("status", proc.status),
                        ("parent", proc.ppid), ("threads", proc.num_threads),
                        ("executable", proc.exe),
                    ):
                        try:
                            setattr(row, attribute, getter())
                        except (psutil.AccessDenied, OSError):
                            row.accessible = False
                    try:
                        row.command = subprocess.list2cmdline(proc.cmdline())
                        priority = proc.nice()
                        row.priority = getattr(priority, "name", str(priority)).replace("_PRIORITY_CLASS", "").replace("_", " ").title()
                    except (psutil.AccessDenied, OSError):
                        row.accessible = False
                    try:
                        mem = proc.memory_info()
                        row.memory = mem.rss
                        row.private = getattr(mem, "private", mem.vms)
                        row.handles = proc.num_handles() if os.name == "nt" else proc.num_fds()
                    except (psutil.AccessDenied, OSError):
                        row.accessible = False
                    try:
                        times = proc.cpu_times()
                        row.cpu_time = times.user + times.system
                        row.cpu = min(100., proc.cpu_percent(None) / cpu_count)
                    except (psutil.AccessDenied, OSError):
                        row.accessible = False
                    try:
                        io = proc.io_counters()
                        row.read_bytes, row.write_bytes = io.read_bytes, io.write_bytes
                    except (psutil.AccessDenied, OSError):
                        row.accessible = False
                if os.name == "nt":
                    from vanta.infra.windows_tasks import process_session
                    row.session = process_session(pid)
                old = self._previous.get(row.identity)
                if old:
                    cpu_delta = max(0, row.cpu_time - old[0])
                    read_delta = max(0, row.read_bytes - old[1])
                    write_delta = max(0, row.write_bytes - old[2])
                    row.read_rate, row.write_rate = read_delta / elapsed, write_delta / elapsed
                    # Count only deltas observed during this VAnta session, never a
                    # process's pre-existing lifetime use or an unobserved restart gap.
                    if row.executable:
                        self.db.execute("INSERT INTO app_usage VALUES (?, ?, ?, ?, ?) ON CONFLICT(application) DO UPDATE SET cpu_seconds=cpu_seconds+excluded.cpu_seconds, read_bytes=read_bytes+excluded.read_bytes, write_bytes=write_bytes+excluded.write_bytes, last_seen=excluded.last_seen", (row.executable, cpu_delta, read_delta, write_delta, time.time()))
                baseline[row.identity] = (row.cpu_time, row.read_bytes, row.write_bytes)
                rows.append(row)
            except (psutil.NoSuchProcess, psutil.ZombieProcess):
                self._processes.pop(pid, None)
            except (psutil.AccessDenied, OSError):
                rows.append(ProcessRow(pid, 0, f"PID {pid}", status="Access denied", accessible=False))
        self._processes = {p: obj for p, obj in self._processes.items() if p in alive}
        self._previous = baseline
        self.db.commit()
        memory = psutil.virtual_memory()
        cores = psutil.cpu_percent(percpu=True)
        times = psutil.cpu_times_percent()
        frequency = psutil.cpu_freq()
        snap = SystemSnapshot(
            time.time(), sum(cores) / max(1, len(cores)), cores, times.system,
            memory.total, memory.used, memory.available, memory.percent,
            time.time() - psutil.boot_time(), rows,
            memory._asdict(),
            {"Logical processors": cpu_count, "Physical cores": psutil.cpu_count(logical=False),
             "Frequency MHz": round(frequency.current) if frequency else None,
             "Threads": sum(p.threads for p in rows), "Handles": sum(p.handles for p in rows),
             "Processes": len(rows)},
        )
        disks = psutil.disk_io_counters(perdisk=True) or {}
        for name, io in disks.items():
            previous = self._disks.get(name)
            snap.disks.append({"Device": name, "Read B/s": max(0, io.read_bytes - previous.read_bytes) / elapsed if previous else 0,
                               "Write B/s": max(0, io.write_bytes - previous.write_bytes) / elapsed if previous else 0,
                               "Read bytes": io.read_bytes, "Write bytes": io.write_bytes})
        self._disks = disks
        networks = psutil.net_io_counters(pernic=True) or {}
        addresses = psutil.net_if_addrs()
        stats = psutil.net_if_stats()
        for name, io in networks.items():
            previous = self._network.get(name)
            stat = stats.get(name)
            snap.networks.append({"Adapter": name,
                "Receive B/s": max(0, io.bytes_recv - previous.bytes_recv) / elapsed if previous else 0,
                "Send B/s": max(0, io.bytes_sent - previous.bytes_sent) / elapsed if previous else 0,
                "Link Mbps": stat.speed if stat else None, "Status": "Up" if stat and stat.isup else "Down",
                "Addresses": ", ".join(a.address for a in addresses.get(name, []))})
        self._network = networks
        if self._nvml:
            nv = self._nvml
            try:
                for index in range(nv.nvmlDeviceGetCount()):
                    handle = nv.nvmlDeviceGetHandleByIndex(index)
                    gpu = {"GPU": index, "Name": nv.nvmlDeviceGetName(handle)}
                    for key, function in (
                        ("Utilization %", lambda: nv.nvmlDeviceGetUtilizationRates(handle).gpu),
                        ("Memory used", lambda: nv.nvmlDeviceGetMemoryInfo(handle).used),
                        ("Memory total", lambda: nv.nvmlDeviceGetMemoryInfo(handle).total),
                        ("Temperature C", lambda: nv.nvmlDeviceGetTemperature(handle, nv.NVML_TEMPERATURE_GPU)),
                        ("Power W", lambda: round(nv.nvmlDeviceGetPowerUsage(handle) / 1000, 1)),
                        ("Driver", nv.nvmlSystemGetDriverVersion),
                    ):
                        try:
                            value = function()
                            gpu[key] = value.decode() if isinstance(value, bytes) else value
                        except Exception:
                            gpu[key] = None
                    snap.gpus.append(gpu)
            except Exception as exc:
                snap.warnings.append(f"GPU: {exc}")
        elif self._gpu_error:
            snap.warnings.append("NVIDIA GPU telemetry unavailable. Other GPU vendors / NPU / GPU-engine and per-process network counters are not provided by this collector.")
        snap.hardware = self.hardware
        if os.name == "nt":
            from vanta.infra.windows_tasks import memory_counters
            try:
                snap.memory_details.update(memory_counters())
            except OSError as exc:
                snap.warnings.append(f"Windows memory counters: {exc}")
        if self._gpu_counters:
            counters = self._gpu_counters.sample()
            by_pid = {p.pid: p for p in rows}
            engines = {}
            for instance, value in counters.get("engines", {}).items():
                match = re.match(r"pid_(\d+)_(.+)", instance)
                if not match:
                    continue
                pid, engine = int(match[1]), match[2]
                engines[engine] = engines.get(engine, 0) + max(0, value)
                proc = by_pid.get(pid)
                if proc is not None:
                    if proc.gpu is None or value > proc.gpu:
                        proc.gpu, proc.gpu_engine = min(100., max(0, value)), engine
            for key, attr in (("dedicated", "gpu_dedicated"), ("shared", "gpu_shared")):
                for instance, value in counters.get(key, {}).items():
                    match = re.match(r"pid_(\d+)_", instance)
                    proc = by_pid.get(int(match[1])) if match else None
                    if proc is not None:
                        setattr(proc, attr, (getattr(proc, attr) or 0) + int(max(0, value)))
            snap.gpu_engines = [{"Engine (Windows LUID)": name, "Utilization %": round(min(100., value), 2)} for name, value in engines.items()]
        self._last_sample = now
        return snap

    def history(self):
        rows = self.db.execute("SELECT application, cpu_seconds, read_bytes, write_bytes, last_seen FROM app_usage ORDER BY cpu_seconds DESC").fetchall()
        since = self.db.execute("SELECT value FROM metadata WHERE key='since'").fetchone()[0]
        return since, rows

    def reset_history(self):
        with self.db:
            self.db.execute("DELETE FROM app_usage")
            self.db.execute("UPDATE metadata SET value=? WHERE key='since'", (time.time(),))
        self._previous.clear()

    def process(self, identity: ProcessIdentity, *, changing: bool = False) -> psutil.Process:
        proc = psutil.Process(identity.pid)
        if identity.created <= 0 or abs(proc.create_time() - identity.created) > .001:
            raise ValueError("The process has exited or its PID was reused. Select it again.")
        if changing and (identity.pid in (0, 4, os.getpid()) or proc.name().lower() in {
            "system", "registry", "smss.exe", "csrss.exe", "wininit.exe", "winlogon.exe", "lsass.exe", "services.exe"}):
            raise PermissionError("VAnta does not modify itself or core Windows processes.")
        return proc

    def end_process(self, identity: ProcessIdentity, tree: bool = False):
        parent = self.process(identity, changing=True)
        children = parent.children(recursive=True) if tree else []
        # Capture every child's identity before changing any process.
        identities = [ProcessIdentity(p.pid, p.create_time()) for p in reversed(children)]
        for child in identities:
            try:
                self.process(child, changing=True).terminate()
            except psutil.NoSuchProcess:
                pass
        # Revalidate the parent after enumerating children.
        self.process(identity, changing=True).terminate()

    def set_priority(self, identity: ProcessIdentity, value: int):
        self.process(identity, changing=True).nice(value)

    def set_affinity(self, identity: ProcessIdentity, cpus: list[int]):
        if not cpus:
            raise ValueError("Select at least one logical processor.")
        self.process(identity, changing=True).cpu_affinity(cpus)

    def services(self):
        if os.name != "nt":
            return []
        result = []
        for service in psutil.win_service_iter():
            try:
                result.append(service.as_dict())
            except (psutil.Error, OSError):
                continue
        return result
