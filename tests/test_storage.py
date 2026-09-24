from vanta.core.models import ActivityEvent, Job, JobStatus, Priority, ResourceSnapshot
from vanta.core.storage import VantaRepository


def test_repository_round_trips_jobs_and_decoded_event_details(tmp_path) -> None:
    repository = VantaRepository(tmp_path / "vanta.db")
    job = Job("persisted", ["demo"], Priority.HIGH, 384)
    repository.save_job(job)
    repository.add_event(ActivityEvent("queue", "persisted entered queue", job.id, {"source": "test"}))

    loaded = repository.load_jobs()
    events = repository.recent_events()
    repository.close()

    assert len(loaded) == 1
    assert loaded[0].id == job.id
    assert loaded[0].priority is Priority.HIGH
    assert loaded[0].command == ["demo"]
    assert events[0]["details"] == {"source": "test"}


def test_running_job_is_recovered_as_cancelled_on_scheduler_restart(tmp_path) -> None:
    from vanta.core.rules import RuleEngine
    from vanta.core.scheduler import Scheduler

    repository = VantaRepository(tmp_path / "vanta.db")
    job = Job("interrupted", ["demo"], status=JobStatus.RUNNING)
    repository.save_job(job)
    scheduler = Scheduler(RuleEngine(), object(), repository)

    assert scheduler.jobs[0].status is JobStatus.CANCELLED
    assert "Interrupted" in scheduler.jobs[0].status_reason
    assert repository.recent_events()[0]["kind"] == "restore"
    repository.close()


def test_scheduler_stops_persisted_child_before_admitting_new_work(tmp_path) -> None:
    import sys
    import time

    from vanta.core.rules import RuleEngine
    from vanta.core.scheduler import Scheduler
    from vanta.infra.processes import ProcessRunner

    repository = VantaRepository(tmp_path / "vanta.db")
    old_runner = ProcessRunner(tmp_path / "old-logs")
    interrupted = Job(
        "interrupted-child",
        [sys.executable, "-c", "import time; time.sleep(30)"],
        status=JobStatus.RUNNING,
    )
    interrupted.pid = old_runner.start(interrupted)
    repository.save_job(interrupted)

    new_runner = ProcessRunner(tmp_path / "new-logs")
    try:
        scheduler = Scheduler(RuleEngine(), new_runner, repository)
        assert scheduler.recovery_blocked is False
        time.sleep(0.05)
        assert old_runner.poll(interrupted.id) is not None
    finally:
        old_runner.close()
        new_runner.close()
        repository.close()


def test_scheduler_blocks_queue_when_persisted_pid_does_not_match_job(tmp_path) -> None:
    import os

    from vanta.core.rules import RuleEngine
    from vanta.core.scheduler import Scheduler
    from vanta.infra.processes import ProcessRunner

    repository = VantaRepository(tmp_path / "vanta.db")
    interrupted = Job("unknown-child", ["definitely-not-the-running-process"], status=JobStatus.RUNNING)
    interrupted.pid = os.getpid()
    repository.save_job(interrupted)
    runner = ProcessRunner(tmp_path / "logs")
    try:
        scheduler = Scheduler(RuleEngine(), runner, repository)
        assert scheduler.recovery_blocked is True
        assert scheduler.jobs[0].status is JobStatus.RUNNING
        queued = scheduler.submit(Job("must-wait", ["demo"]))
        scheduler.tick(ResourceSnapshot(vram_total_mb=4096, telemetry_available=True))
        assert queued.status is JobStatus.QUEUED
        assert scheduler.cancel(interrupted.id) is False
        assert interrupted.status is JobStatus.RUNNING
    finally:
        runner.close()
        repository.close()
