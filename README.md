# VAnta

VAnta is a user-level GPU workload controller for Windows. It gives GPU jobs a
visible queue, an explainable VRAM admission rule, controlled process launch,
and post-launch verification.

The controller follows one loop:

**Observe → Rule → Queue / Action → Verify**

VAnta runs as a normal desktop application. It does not modify the NVIDIA
driver, Windows Scheduler, registry, or system services.

![VAnta dashboard](docs/images/dashboard.png)

## What it provides

- Live NVIDIA GPU, VRAM, temperature, power, CPU, and RAM telemetry
- Critical, high, normal, and low priority scheduling
- One controlled GPU workload at a time in the live MVP controller
- A 512 MB VRAM safety reserve and explainable admission decisions
- Controlled launch and cancellation of child processes
- Before/after VRAM verification and a decision feed
- SQLite job and event history restored across app restarts
- Safe recovery of interrupted child processes after an unexpected shutdown
- A Policy Lab comparing strict priority with fit-what-fits scheduling
- A one-click demo scenario for queueing, verification, and the VRAM guard
- A built-in CUDA pulse that does not require PyTorch or the CUDA Toolkit

## Quick start for users

1. Download the latest `VAnta-windows-x64.zip` from the [Releases](../../releases) page, extract it, and run `VAnta\VAnta.exe`.
2. Click **Run demo scenario**.
3. Watch the queue and **Decision feed**:
   - the balanced workload starts first;
   - the critical workload waits without interrupting it;
   - the VRAM guard check is rejected when it exceeds safe capacity.
4. Use **Add GPU Pulse to queue** for a light, balanced, or heavy workload.
5. Use **Submit custom command** for a CLI workload you want VAnta to start.
6. Select a job and click **Cancel selected** to stop a queued or running job.
7. Open **Policy Lab** to compare the two queue policies with a deterministic
   replay.

The built-in CUDA pulse requires a working NVIDIA driver and an NVIDIA GPU.
The dashboard can still open without one, but telemetry and CUDA pulse jobs
will be unavailable. VAnta does not install a driver or the CUDA Toolkit.

## Installation from source

### Requirements

- Windows 10 or Windows 11
- Python 3.11 or newer, or [uv](https://docs.astral.sh/uv/)
- An NVIDIA GPU and current NVIDIA driver for live GPU telemetry and the CUDA
  pulse

Install `uv` in PowerShell if it is not already installed:

```powershell
winget install --id=astral-sh.uv -e
```

Clone and run VAnta:

```powershell
git clone https://github.com/pllpert643-ship-it/VAnta.git
cd VAnta
uv sync --extra dev
uv run vanta
```

`uv sync` creates the project environment and installs the exact locked
dependencies from `uv.lock`. No global Python package installation is needed.

## Build the Windows application

Run PowerShell from the repository root:

```powershell
.\scripts\build.ps1
```

The script installs the locked dependencies, runs PyInstaller, and creates:

```text
dist\VAnta\VAnta.exe
```

The packaged application runs at normal user level. It does not install a
Windows service. To test the packaged executable without opening the full UI:

```powershell
.\dist\VAnta\VAnta.exe --help
```

## Verify a checkout

```powershell
uv run pytest -q
```

The test suite covers admission rules, scheduling order, process recovery,
persistence, the demo scenario, and the Policy Lab.

## Local data and logs

VAnta stores its local SQLite database and child-process logs under:

```text
%LOCALAPPDATA%\VAnta\
```

Deleting that directory resets local job and decision history. Running jobs
should be stopped from the app before removing it.

## Troubleshooting

**The header says `GPU telemetry unavailable`.** Check that an NVIDIA GPU and
driver are installed. VAnta uses NVML through `nvidia-ml-py`; the CUDA Toolkit
is not required.

**A CUDA pulse exits immediately.** Check the process log under
`%LOCALAPPDATA%\VAnta\logs\`. The pulse needs `nvcuda.dll`, which is provided by
the NVIDIA display driver.

**The queue says `QUEUE PAUSED`.** VAnta could not confirm that a workload from
an interrupted previous session stopped. Close or stop that process, then
restart VAnta so recovery can be checked again.

## Project documents

- [Project brief](docs/PROJECT.md)
- [Demo script](docs/DEMO_SCRIPT.md)
- [License](LICENSE)


