from vanta.core.models import Priority
from vanta.core.policy_lab import QueuePolicy, SimulationJob, simulate


def scenario() -> list[SimulationJob]:
    return [
        SimulationJob("warm-up", Priority.NORMAL, 512, 6, 0),
        SimulationJob("critical", Priority.CRITICAL, 800, 4, 1),
        SimulationJob("preview", Priority.LOW, 256, 2, 1),
    ]


def test_strict_priority_preserves_order_when_head_waits_for_capacity() -> None:
    result = simulate(scenario(), policy=QueuePolicy.STRICT_PRIORITY)

    assert result.order == ("warm-up", "critical", "preview")
    assert result.records[2].wait_s == 9


def test_fit_what_fits_reduces_small_job_wait_without_breaking_reserve() -> None:
    result = simulate(scenario(), policy=QueuePolicy.FIT_WHAT_FITS)

    assert result.order == ("warm-up", "preview", "critical")
    assert result.records[1].wait_s == 0
    assert result.average_wait_s < simulate(scenario()).average_wait_s


def test_policy_lab_uses_single_job_mvp_limit_and_arrival_order() -> None:
    result = simulate(
        [
            SimulationJob("blocker", Priority.HIGH, 512, 5, 0),
            SimulationJob("Z-first", Priority.NORMAL, 512, 1, 1),
            SimulationJob("A-later", Priority.NORMAL, 512, 1, 2),
        ],
        max_concurrent_jobs=1,
    )

    assert result.order == ("blocker", "Z-first", "A-later")
    assert result.records[1].start_s == 5
