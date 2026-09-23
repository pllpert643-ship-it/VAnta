# VAnta

VAnta is a user-level GPU workload controller for Windows, built for the
InfinityX Global Hackathon 2K26.

It follows a transparent control loop:

**Observe → Rule → Queue / Action → Verify**

VAnta monitors local resources, applies explainable admission rules, schedules
jobs submitted through the application, and records what changed after every
action. It does not modify the NVIDIA driver or Windows scheduler.

![VAnta dashboard](docs/images/dashboard.png)

## What the MVP demonstrates

- Live GPU, VRAM, CPU, RAM, temperature, and power telemetry
- Critical-to-low priority scheduling with a one-job concurrency guard
- VRAM admission decisions with a 512 MB safety reserve
- Controlled launch and cancellation of child processes
- A visible decision feed with post-launch verification
- A built-in CUDA pulse for repeatable demos without PyTorch

## Development

```powershell
uv sync --extra dev
uv run vanta
```

The included GPU Pulse workload uses the installed NVIDIA driver directly, so
the demo does not require PyTorch or the CUDA Toolkit.

## Build the Windows application

```powershell
.\scripts\build.ps1
```

The packaged application is created at `dist\VAnta\VAnta.exe`. It runs at
normal user level and does not install a Windows service or modify the driver.

## Project material

- [Project brief](docs/PROJECT.md)
- [Demo video script](docs/DEMO_SCRIPT.md)
