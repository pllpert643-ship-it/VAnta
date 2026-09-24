from vanta.core.demo import build_demo_specs
from vanta.core.models import Priority, ResourceSnapshot


def test_demo_specs_cover_start_wait_and_vram_guard() -> None:
    specs = build_demo_specs(
        ResourceSnapshot(vram_total_mb=4096, telemetry_available=True)
    )

    assert [spec.priority for spec in specs] == [Priority.NORMAL, Priority.CRITICAL, Priority.LOW]
    assert specs[0].required_vram_mb == 512
    assert specs[2].required_vram_mb == 3585


def test_demo_guard_request_is_safe_when_gpu_capacity_is_unknown() -> None:
    specs = build_demo_specs(ResourceSnapshot(telemetry_available=False))

    # The recipe remains valid without a GPU; the rule engine will use its
    # telemetry-unavailable fallback and the pulse can still demonstrate the
    # process lifecycle.
    assert specs[-1].required_vram_mb >= 2049
