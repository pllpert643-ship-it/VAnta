from __future__ import annotations

import argparse


def main() -> int:
    parser = argparse.ArgumentParser(prog="VAnta")
    parser.add_argument("--gpu-workload", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--memory-mb", type=int, default=256, help=argparse.SUPPRESS)
    parser.add_argument("--duration", type=int, default=20, help=argparse.SUPPRESS)
    args = parser.parse_args()

    if args.gpu_workload:
        from vanta.workloads.gpu_pulse import run

        return run(memory_mb=args.memory_mb, duration=args.duration)

    from vanta.app import run_app

    return run_app()


if __name__ == "__main__":
    raise SystemExit(main())
