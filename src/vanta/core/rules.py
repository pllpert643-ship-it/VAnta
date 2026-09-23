from __future__ import annotations

from dataclasses import dataclass
from enum import Enum

from vanta.core.models import Job, ResourceSnapshot


class RuleAction(str, Enum):
    START = "start"
    WAIT = "wait"
    REJECT = "reject"


@dataclass(frozen=True, slots=True)
class RuleDecision:
    action: RuleAction
    reason: str


@dataclass(slots=True)
class RuleConfig:
    max_concurrent_jobs: int = 1
    reserved_vram_mb: int = 512


class RuleEngine:
    def __init__(self, config: RuleConfig | None = None) -> None:
        self.config = config or RuleConfig()

    def evaluate(
        self,
        job: Job,
        snapshot: ResourceSnapshot,
        running_count: int,
    ) -> RuleDecision:
        if running_count >= self.config.max_concurrent_jobs:
            return RuleDecision(
                RuleAction.WAIT,
                f"Concurrency limit reached ({self.config.max_concurrent_jobs})",
            )

        if not snapshot.telemetry_available:
            return RuleDecision(
                RuleAction.START,
                "GPU telemetry unavailable; concurrency guard allows this job",
            )

        usable_capacity = max(0, snapshot.vram_total_mb - self.config.reserved_vram_mb)
        if job.required_vram_mb > usable_capacity:
            return RuleDecision(
                RuleAction.REJECT,
                f"Requests {job.required_vram_mb} MB; safe capacity is {usable_capacity:.0f} MB",
            )

        required_free = job.required_vram_mb + self.config.reserved_vram_mb
        if snapshot.vram_free_mb < required_free:
            return RuleDecision(
                RuleAction.WAIT,
                f"Waiting for {required_free:.0f} MB free VRAM; {snapshot.vram_free_mb:.0f} MB available",
            )

        return RuleDecision(
            RuleAction.START,
            f"Rule passed: {snapshot.vram_free_mb:.0f} MB VRAM free",
        )
