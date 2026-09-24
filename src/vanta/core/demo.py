"""Deterministic workload recipes used by the interactive demo."""

from __future__ import annotations

from dataclasses import dataclass

from vanta.core.models import Priority, ResourceSnapshot


@dataclass(frozen=True, slots=True)
class DemoJobSpec:
    """A small, presentation-friendly description of one demo job."""

    name: str
    priority: Priority
    pulse_memory_mb: int
    duration_s: int
    required_vram_mb: int


def build_demo_specs(snapshot: ResourceSnapshot) -> tuple[DemoJobSpec, ...]:
    """Return a repeatable scenario that exercises the main MVP decisions.

    The normal job is submitted first by the UI so that it can start. The
    critical job is then queued behind it, demonstrating non-preemptive
    priority scheduling. The final job requests one megabyte above the safe
    capacity and is rejected when telemetry is available.
    """

    safe_capacity = max(0, int(snapshot.vram_total_mb) - 512)
    guard_request = max(2049, safe_capacity + 1)
    return (
        DemoJobSpec("Demo · Balanced workload", Priority.NORMAL, 512, 12, 512),
        DemoJobSpec("Demo · Critical workload", Priority.CRITICAL, 256, 5, 256),
        DemoJobSpec("Demo · VRAM guard check", Priority.LOW, 256, 2, guard_request),
    )
