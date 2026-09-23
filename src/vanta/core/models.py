from __future__ import annotations

from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from enum import Enum, IntEnum
from typing import Any
from uuid import uuid4


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


class Priority(IntEnum):
    LOW = 10
    NORMAL = 20
    HIGH = 30
    CRITICAL = 40

    @property
    def label(self) -> str:
        return self.name.title()


class JobStatus(str, Enum):
    QUEUED = "queued"
    WAITING = "waiting"
    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"
    CANCELLED = "cancelled"


@dataclass(slots=True)
class ResourceSnapshot:
    timestamp: str = field(default_factory=utc_now)
    gpu_name: str = "NVIDIA GPU"
    gpu_utilization: float = 0.0
    vram_used_mb: float = 0.0
    vram_total_mb: float = 0.0
    temperature_c: float | None = None
    power_w: float | None = None
    cpu_percent: float = 0.0
    ram_percent: float = 0.0
    telemetry_available: bool = False

    @property
    def vram_free_mb(self) -> float:
        return max(0.0, self.vram_total_mb - self.vram_used_mb)

    @property
    def vram_percent(self) -> float:
        if self.vram_total_mb <= 0:
            return 0.0
        return (self.vram_used_mb / self.vram_total_mb) * 100

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(slots=True)
class Job:
    name: str
    command: list[str]
    priority: Priority = Priority.NORMAL
    required_vram_mb: int = 256
    id: str = field(default_factory=lambda: uuid4().hex[:10])
    status: JobStatus = JobStatus.QUEUED
    created_at: str = field(default_factory=utc_now)
    started_at: str | None = None
    finished_at: str | None = None
    pid: int | None = None
    exit_code: int | None = None
    status_reason: str = "Waiting for scheduler"
    verified: bool = False
    before_snapshot: dict[str, Any] | None = None

    def to_dict(self) -> dict[str, Any]:
        data = asdict(self)
        data["priority"] = int(self.priority)
        data["status"] = self.status.value
        return data


@dataclass(slots=True)
class ActivityEvent:
    kind: str
    message: str
    job_id: str | None = None
    details: dict[str, Any] = field(default_factory=dict)
    timestamp: str = field(default_factory=utc_now)
