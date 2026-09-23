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
