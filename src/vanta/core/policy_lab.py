from __future__ import annotations

from dataclasses import dataclass
from enum import Enum

from vanta.core.models import Priority


class QueuePolicy(str, Enum):
    STRICT_PRIORITY = "strict_priority"
    FIT_WHAT_FITS = "fit_what_fits"

    @property
    def label(self) -> str:
        return {
            QueuePolicy.STRICT_PRIORITY: "Strict priority",
            QueuePolicy.FIT_WHAT_FITS: "Fit what fits",
        }[self]


@dataclass(frozen=True, slots=True)
class SimulationJob:
    name: str
    priority: Priority
    required_vram_mb: int
    duration_s: float
    submitted_at_s: float = 0.0


@dataclass(frozen=True, slots=True)
class SimulationRecord:
    name: str
    start_s: float
    finish_s: float
    wait_s: float


@dataclass(frozen=True, slots=True)
class SimulationResult:
    policy: QueuePolicy
    records: tuple[SimulationRecord, ...]
    rejected: tuple[str, ...]
    makespan_s: float
    average_wait_s: float
    utilization_pct: float

    @property
    def order(self) -> tuple[str, ...]:
        return tuple(record.name for record in sorted(self.records, key=lambda item: item.start_s))


def simulate(
    jobs: list[SimulationJob],
    *,
    total_vram_mb: int = 1536,
    reserved_vram_mb: int = 512,
    max_concurrent_jobs: int = 2,
    policy: QueuePolicy = QueuePolicy.STRICT_PRIORITY,
) -> SimulationResult:
    """Run a deterministic queue simulation for the Policy Lab.

    The simulator models arrivals, VRAM admission, concurrency, and completion.
    It never starts a process or reads live machine state.

    The Policy Lab intentionally uses two hypothetical slots by default so the
    UI can compare strict priority with fit-what-fits. The live MVP controller
    remains configured for one controlled workload at a time.
    """
    safe_capacity = max(0, total_vram_mb - reserved_vram_mb)
    pending = [
        (sequence, job)
        for sequence, job in enumerate(jobs)
        if job.required_vram_mb <= safe_capacity
    ]
    rejected = tuple(job.name for job in jobs if job.required_vram_mb > safe_capacity)
    pending.sort(key=lambda item: (item[1].submitted_at_s, -int(item[1].priority), item[0]))
    running: list[tuple[SimulationJob, float]] = []
    records: list[SimulationRecord] = []
    time_s = 0.0

    while pending or running:
        arrived = [item for item in pending if item[1].submitted_at_s <= time_s]
        pending = [item for item in pending if item[1].submitted_at_s > time_s]
        waiting = arrived

        finished = [(job, end) for job, end in running if end <= time_s]
        running = [(job, end) for job, end in running if end > time_s]

        # Jobs that arrived earlier remain ahead of future arrivals. Within an
        # arrival batch, both policies use priority and then name as a stable tie-break.
        waiting.sort(key=lambda item: (-int(item[1].priority), item[1].submitted_at_s, item[0]))
        while waiting and len(running) < max_concurrent_jobs:
            used_vram = sum(job.required_vram_mb for job, _ in running)
            if policy is QueuePolicy.STRICT_PRIORITY:
                candidate = waiting[0][1]
                if used_vram + candidate.required_vram_mb > safe_capacity:
                    break
            else:
                candidate_item = next(
                    (
                        item
                        for item in waiting
                        if used_vram + item[1].required_vram_mb <= safe_capacity
                    ),
                    None,
                )
                if candidate_item is None:
                    break
                candidate = candidate_item[1]
            waiting.remove(next(item for item in waiting if item[1] is candidate))
            finish_s = time_s + max(0.0, candidate.duration_s)
            running.append((candidate, finish_s))
            records.append(
                SimulationRecord(
                    name=candidate.name,
                    start_s=time_s,
                    finish_s=finish_s,
                    wait_s=max(0.0, time_s - candidate.submitted_at_s),
                )
            )

        # Keep unstarted arrivals in the pending queue. They are considered again
        # after the next completion or arrival event.
        pending.extend(waiting)
        pending.sort(key=lambda item: (item[1].submitted_at_s, -int(item[1].priority), item[0]))

        next_times = [end for _, end in running]
        next_times.extend(item[1].submitted_at_s for item in pending if item[1].submitted_at_s > time_s)
        if not next_times:
            break
        next_time = min(next_times)
        if next_time <= time_s:
            time_s += 0.001
        else:
            time_s = next_time

    makespan_s = max((record.finish_s for record in records), default=0.0)
    average_wait_s = sum(record.wait_s for record in records) / len(records) if records else 0.0
    busy_seconds = sum(record.finish_s - record.start_s for record in records)
    utilization_pct = (
        min(100.0, busy_seconds / (makespan_s * max(1, max_concurrent_jobs)) * 100)
        if makespan_s
        else 0.0
    )
    return SimulationResult(
        policy=policy,
        records=tuple(records),
        rejected=rejected,
        makespan_s=makespan_s,
        average_wait_s=average_wait_s,
        utilization_pct=utilization_pct,
    )
