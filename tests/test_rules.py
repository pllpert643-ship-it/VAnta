from vanta.core.models import Job, ResourceSnapshot
from vanta.core.rules import RuleAction, RuleConfig, RuleEngine


def snapshot(*, used: int = 500, total: int = 4096, available: bool = True) -> ResourceSnapshot:
    return ResourceSnapshot(
        vram_used_mb=used,
        vram_total_mb=total,
        telemetry_available=available,
    )


def test_starts_when_vram_and_slot_are_available() -> None:
    engine = RuleEngine(RuleConfig(max_concurrent_jobs=1, reserved_vram_mb=512))
    decision = engine.evaluate(Job("job", ["demo"], required_vram_mb=512), snapshot(), 0)
    assert decision.action is RuleAction.START

def test_waits_when_concurrency_slot_is_busy() -> None:
    engine = RuleEngine(RuleConfig(max_concurrent_jobs=1, reserved_vram_mb=512))
    decision = engine.evaluate(Job("job", ["demo"]), snapshot(), 1)
    assert decision.action is RuleAction.WAIT


def test_rejects_job_larger_than_safe_capacity() -> None:
    engine = RuleEngine(RuleConfig(reserved_vram_mb=512))
    decision = engine.evaluate(Job("job", ["demo"], required_vram_mb=3800), snapshot(), 0)
    assert decision.action is RuleAction.REJECT


def test_falls_back_to_concurrency_guard_without_telemetry() -> None:
    engine = RuleEngine()
    decision = engine.evaluate(Job("job", ["demo"]), snapshot(available=False), 0)
    assert decision.action is RuleAction.START
