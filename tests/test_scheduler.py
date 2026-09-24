from vanta.core.models import Job, JobStatus, Priority, ResourceSnapshot
from vanta.core.rules import RuleEngine
from vanta.core.scheduler import Scheduler


class FakeRepository:
    def __init__(self) -> None:
        self.events = []

    def save_job(self, job: Job) -> None:
        pass

    def add_event(self, event: object) -> None:
        self.events.append(event)


class FakeRunner:
    def __init__(self) -> None:
        self.started: list[str] = []

    def start(self, job: Job) -> int:
        self.started.append(job.id)
        return 4242

    def poll(self, job_id: str) -> None:
        return None

    def cancel(self, job_id: str) -> bool:
        return True


class LifecycleRunner(FakeRunner):
    def __init__(self) -> None:
        super().__init__()
        self.polls: dict[str, int] = {}

    def start(self, job: Job) -> int:
        pid = super().start(job)
        self.polls[job.id] = 0
        return pid

    def poll(self, job_id: str) -> int | None:
        self.polls[job_id] += 1
        return 0 if self.polls[job_id] >= 2 else None


def test_higher_priority_job_starts_first_and_others_wait() -> None:
    runner = FakeRunner()
    scheduler = Scheduler(RuleEngine(), runner, FakeRepository())
    normal = Job("normal", ["demo"], Priority.NORMAL, 256)
    critical = Job("critical", ["demo"], Priority.CRITICAL, 256)
    scheduler.submit(normal)
    scheduler.submit(critical)
    metrics = ResourceSnapshot(
        vram_used_mb=200,
        vram_total_mb=4096,
        telemetry_available=True,
    )

    scheduler.tick(metrics)
    scheduler.tick(metrics)

    assert runner.started == [critical.id]
    assert critical.status is JobStatus.RUNNING
    assert normal.status is JobStatus.WAITING
    assert "Concurrency limit" in normal.status_reason


def test_strict_priority_blocks_a_lower_job_when_head_waits_for_vram() -> None:
    runner = FakeRunner()
    scheduler = Scheduler(RuleEngine(), runner, FakeRepository())
    high = Job("high-memory", ["demo"], Priority.HIGH, 512)
    low = Job("small", ["demo"], Priority.LOW, 128)
    scheduler.submit(high)
    scheduler.submit(low)
    metrics = ResourceSnapshot(
        vram_used_mb=3400,
        vram_total_mb=4096,
        telemetry_available=True,
    )

    scheduler.tick(metrics)

    assert runner.started == []
    assert high.status is JobStatus.WAITING
    assert low.status is JobStatus.QUEUED


def test_scheduler_runs_waiting_job_after_current_job_completes() -> None:
    runner = LifecycleRunner()
    scheduler = Scheduler(RuleEngine(), runner, FakeRepository())
    normal = Job("normal", ["demo"], Priority.NORMAL, 256)
    critical = Job("critical", ["demo"], Priority.CRITICAL, 256)
    metrics = ResourceSnapshot(vram_total_mb=4096, telemetry_available=True)

    scheduler.submit(normal)
    scheduler.tick(metrics)
    scheduler.submit(critical)
    scheduler.tick(metrics)
    assert normal.status is JobStatus.RUNNING
    assert critical.status is JobStatus.WAITING

    scheduler.tick(metrics)
    scheduler.tick(metrics)

    assert normal.status is JobStatus.COMPLETED
    assert critical.status is JobStatus.RUNNING
    assert runner.started == [normal.id, critical.id]


def test_scheduler_rejects_job_above_safe_vram_capacity() -> None:
    runner = FakeRunner()
    repository = FakeRepository()
    scheduler = Scheduler(RuleEngine(), runner, repository)
    oversized = Job("oversized", ["demo"], Priority.NORMAL, 3585)

    scheduler.submit(oversized)
    scheduler.tick(
        ResourceSnapshot(
            vram_total_mb=4096,
            vram_used_mb=0,
            telemetry_available=True,
        )
    )

    assert oversized.status is JobStatus.FAILED
    assert "safe capacity is 3584" in oversized.status_reason
    assert runner.started == []
