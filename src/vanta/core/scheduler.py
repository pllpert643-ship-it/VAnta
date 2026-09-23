from __future__ import annotations

from collections.abc import Callable
from datetime import datetime, timezone

from vanta.core.models import ActivityEvent, Job, JobStatus, ResourceSnapshot, utc_now
from vanta.core.rules import RuleAction, RuleEngine
from vanta.core.storage import VantaRepository
from vanta.infra.processes import ProcessRunner


EventSink = Callable[[ActivityEvent], None]


class Scheduler:
    def __init__(
        self,
        rules: RuleEngine,
        runner: ProcessRunner,
        repository: VantaRepository,
        event_sink: EventSink | None = None,
    ) -> None:
        self.rules = rules
        self.runner = runner
        self.repository = repository
        self.event_sink = event_sink
        self.jobs: list[Job] = []

    def submit(self, job: Job) -> Job:
        self.jobs.append(job)
        self.repository.save_job(job)
        self._emit("queue", f"{job.name} entered the {job.priority.label.lower()} priority queue", job)
        return job

    def tick(self, snapshot: ResourceSnapshot) -> None:
        self._poll_running(snapshot)
        running_count = sum(job.status is JobStatus.RUNNING for job in self.jobs)
        candidates = sorted(
            (job for job in self.jobs if job.status in {JobStatus.QUEUED, JobStatus.WAITING}),
            key=lambda job: (-int(job.priority), job.created_at),
        )

        for job in candidates:
            decision = self.rules.evaluate(job, snapshot, running_count)
            if decision.action is RuleAction.WAIT:
                if job.status is not JobStatus.WAITING or job.status_reason != decision.reason:
                    job.status = JobStatus.WAITING
                    job.status_reason = decision.reason
                    self.repository.save_job(job)
                continue

            if decision.action is RuleAction.REJECT:
                job.status = JobStatus.FAILED
                job.status_reason = decision.reason
                job.finished_at = utc_now()
                self.repository.save_job(job)
                self._emit("rule", f"Rejected {job.name}: {decision.reason}", job)
                continue

            try:
                job.pid = self.runner.start(job)
            except Exception as exc:
                job.status = JobStatus.FAILED
                job.status_reason = f"Could not start: {exc}"
                job.finished_at = utc_now()
                self.repository.save_job(job)
                self._emit("error", f"Could not start {job.name}: {exc}", job)
                continue

            job.status = JobStatus.RUNNING
            job.status_reason = decision.reason
            job.started_at = utc_now()
            job.before_snapshot = snapshot.to_dict()
            self.repository.save_job(job)
            self._emit("action", f"Started {job.name} (PID {job.pid}): {decision.reason}", job)
            running_count += 1
            if running_count >= self.rules.config.max_concurrent_jobs:
                break

    def _poll_running(self, snapshot: ResourceSnapshot) -> None:
        for job in [item for item in self.jobs if item.status is JobStatus.RUNNING]:
            return_code = self.runner.poll(job.id)
            if return_code is not None:
                job.exit_code = return_code
                job.finished_at = utc_now()
                job.status = JobStatus.COMPLETED if return_code == 0 else JobStatus.FAILED
                job.status_reason = (
                    "Completed successfully" if return_code == 0 else f"Exited with code {return_code}"
                )
                self.repository.save_job(job)
                self._emit(
                    "complete" if return_code == 0 else "error",
                    f"{job.name} {job.status_reason.lower()}",
                    job,
                )
                continue

            if not job.verified and job.started_at:
                started = datetime.fromisoformat(job.started_at)
                elapsed = (datetime.now(timezone.utc) - started).total_seconds()
                if elapsed >= 2:
                    job.verified = True
                    self.repository.save_job(job)
                    before = job.before_snapshot or {}
                    delta = snapshot.vram_used_mb - float(before.get("vram_used_mb", 0))
                    self._emit(
                        "verify",
                        f"Verified {job.name}: VRAM changed by {delta:+.0f} MB",
                        job,
                        {"vram_delta_mb": round(delta, 1), "gpu_utilization": snapshot.gpu_utilization},
                    )

    def cancel(self, job_id: str) -> bool:
        job = next((item for item in self.jobs if item.id == job_id), None)
        if job is None or job.status in {JobStatus.COMPLETED, JobStatus.FAILED, JobStatus.CANCELLED}:
            return False
        if job.status is JobStatus.RUNNING:
            self.runner.cancel(job.id)
        job.status = JobStatus.CANCELLED
        job.status_reason = "Cancelled by user"
        job.finished_at = utc_now()
        self.repository.save_job(job)
        self._emit("cancel", f"Cancelled {job.name}", job)
        return True

    def _emit(
        self,
        kind: str,
        message: str,
        job: Job,
        details: dict[str, object] | None = None,
    ) -> None:
        event = ActivityEvent(kind=kind, message=message, job_id=job.id, details=details or {})
        self.repository.add_event(event)
        if self.event_sink:
            self.event_sink(event)
