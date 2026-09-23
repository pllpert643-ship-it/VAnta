# VAnta — Project Brief

## Event

InfinityX Global Hackathon 2K26

## Project name

**VAnta**

User-Level GPU Workload Controller for Windows

## Problem statement

GPU workloads are often launched as unrelated processes. On a workstation with
limited VRAM, jobs can start at the same time, compete for memory, fail
unpredictably, and give the user no explanation of why one workload ran before
another. Existing system-level scheduling is difficult to change safely and is
outside the reach of most application developers.

## Solution

VAnta provides an explainable control layer for workloads submitted through the
application. It monitors live resources, evaluates simple rules, places jobs in
a priority queue, launches them when safe, and verifies the resource change
after every action.

The control loop is:

> Observe → Rule → Queue / Action → Verify

VAnta works at normal user level. It does not modify the NVIDIA driver, the
Windows scheduler, or the registry.

## Key features

- Live NVIDIA GPU utilization, VRAM, temperature, and power telemetry
- CPU and system RAM monitoring
- Explainable VRAM admission rule with a configurable safety reserve
- Critical, high, normal, and low priority levels
- Predictable non-preemptive scheduling
- Controlled process launch and cancellation
- Before/after verification with measured VRAM change
- Decision feed that records why each action happened
- SQLite event history stored locally
- Built-in CUDA workload that requires no PyTorch or CUDA Toolkit
- Custom command submission for real CLI workloads

## Technology stack

| Layer | Technology | Purpose |
| --- | --- | --- |
| Desktop UI | Python, PySide6 / Qt | Native Windows dashboard |
| GPU telemetry | NVIDIA Management Library (`pynvml`) | GPU, VRAM, temperature, power |
| Host telemetry | `psutil` | CPU and RAM readings |
| Scheduling | Python domain modules | Rules, priority ordering, lifecycle |
| Persistence | SQLite | Jobs and decision history |
| Demo workload | CUDA Driver API through `ctypes` | Real VRAM allocation and GPU activity |
| Packaging | PyInstaller | Standalone Windows application |
| Environment | uv | Reproducible dependency management |
| Testing | pytest | Rule and scheduler regression tests |

## MVP scope

- Windows 11
- One NVIDIA GPU
- Jobs launched through VAnta
- One controlled GPU workload at a time
- 512 MB VRAM safety reserve
- External GPU processes are observed but not controlled

## Why this design is safe

VAnta only manages child processes that it starts. The default scheduler lets a
running workload finish and applies priority ordering to jobs that are still
waiting. This avoids unsafe suspension of an arbitrary CUDA process and loss of
its GPU context.

## Product direction

The provider boundary can later support multiple GPUs, Linux telemetry, remote
workers, per-team quotas, schedules, and checkpoint-aware preemption. The MVP
keeps the policy visible and testable before expanding control authority.
