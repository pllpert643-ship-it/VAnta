# VAnta Native

VAnta Native is the Rust desktop migration of VAnta. The Python edition remains available while the migration is validated. Both editions use separate data directories; native startup never rewrites the Python SQLite history.

## Run

Extract the complete `VAnta-native-windows-x64.zip` folder and run `VAntaNative.exe`. The native application uses OpenGL through eframe/egui and loads NVIDIA NVML only when available. Windows 10/11 x64 and a working graphics driver are required. NVIDIA GPU features need the installed NVIDIA driver. CPU, RAM and process observations remain usable without NVIDIA.


## Store edition (0.2.1 release candidate)

Build with `scripts/build-native.ps1 -Store`. The Store edition uses a separate `VAntaStore` local data directory, includes the privacy policy in Settings, and retains resource observations, process actions with confirmation, memory snapshots, the workload queue and Policy Lab. Services, startup and system memory administration are read-only; backend calls reject service control, StartupApproved writes and all system cache/standby mutations. Startup enabled state is reported as unknown instead of interpreting Explorer's undocumented approval records. This edition does not request elevation. Optional NVIDIA DLLs are located using the Windows system-directory API; NVML never searches the current directory for nvml.dll.

The Store payload is `dist/VAnta-Store/`; its ZIP is a portable test payload, not an installer. See `docs/STORE_RELEASE.md` for the separate MSIX packaging procedure, real Partner Center identity requirements and remaining installation/certification gates. The sections below describe portable administration where a feature is unavailable in the Store edition.

## Workspaces

- **Resource overview:** CPU, physical RAM, NVIDIA GPU/VRAM, a bounded timeline, largest working sets and observations about resource pressure. It does not diagnose hardware failure.
- **GPU workloads:** native CUDA pulse and custom executable jobs; literal JSON argument arrays; strict priority with one controlled child and a 512 MiB reserve. Fresh GPU observations are required by default. Declared VRAM is an estimate, not an enforced memory cap.
- **Task manager:** search and numeric sorting, process CPU normalized to total machine capacity, working sets and interval I/O bytes (including non-disk I/O on Windows). Select a process to inspect mappings, adjust safe CPU priority classes or a single-group affinity mask, trim its working set or terminate it after confirmation. Full creation timestamps are pinned on live selection and rechecked on the action handle.
- **Memory map:** physical RAM, available memory, swap, Windows commit/cache/kernel counters, selected-process virtual regions, native JSON snapshot save/open, and process CSV export. Working sets can overlap; virtual mappings are not resident RAM.
- **Policy lab:** deterministic strict-priority versus fit-what-fits simulation. Editing its workload sizes and durations changes a virtual timeline; no workload executes.
- **Administration:** explicit live inventory of local services and startup entries; confirmed service start/stop/restart; supported current-user Run approval changes; confirmed system-cache trim and advanced standby/priority-zero purge. System memory maintenance requires an already elevated launch and a memory/elevation capture from the last 60 seconds.
- **Settings:** capture interval, bounded history, reduced motion and native migration status. The default motion choice follows Windows animation settings; page fades, hover transitions and smoothly changing metric bars stop under Reduce motion.

## Jobs and recovery

Only one GUI may use a data directory at a time; an OS file handle prevents concurrent queue writes and is released even after process exit. Native queue state and logs live under the native local application-data directory. Settings shows the directory. `VANTA_NATIVE_DATA_DIR` selects an isolated location for testing. Queue writes replace the old document only after the temporary file is flushed. Corrupt queue storage is preserved and blocks dispatch instead of silently resetting history.

Built-in GPU pulse entries resolve the current executable when dispatched, so a package upgrade does not leave newly queued built-in jobs pointing at the old installation path. Custom executables keep their explicitly chosen paths. After restart, persisted running jobs become interrupted. Queue admission stays paused until you explicitly resume it. Persisted PIDs are never used to terminate a process. On Windows, workloads are created with their primary thread suspended. The controller assigns the child to a kill-on-close Job Object, starts bounded log readers, verifies membership, and resumes the actual primary thread. Assignment or resume failure cancels the owned child. The controller never searches persisted PIDs to reclaim processes. Log readers stop without waiting forever for an inherited writer. Native stdout/stderr logs retain a bounded amount per stream and the decision feed and terminal history are bounded.

After two seconds, a still-running job can receive one post-launch observation comparing fresh global VRAM against its admission baseline. It reports an observed device-wide delta, not the amount allocated by that job. Jobs that finish earlier or lack a valid baseline do not receive invented verification.

The CUDA pulse allocates VRAM and repeatedly fills it. It is not an FPS or general GPU performance benchmark. The program does not install drivers, change clocks or launch workloads automatically on first start.

## Snapshots and CSV

Native snapshots use their own versioned JSON schema. They do not import Python Memory Map JSON, `.RMP` files or SQLite databases. Historical mode disables process actions and clearly labels observations. Snapshot validation rejects unsupported schemas, duplicate PIDs, invalid counters and oversized input. Export includes process names and executable paths; choose what to share. Spreadsheet-dangerous process names are escaped in CSV.

## Build and checks

Install Rust 1.100 or later using rustup. On Windows, use MSVC with Visual Studio C++ Build Tools, or the GNU toolchain with MinGW-w64 `gcc` on PATH. The local project configuration uses `gcc` for the GNU target. From the repository root:

```powershell
.\scripts\build-native.ps1
```

The script runs tests and creates `dist\VAnta-Native\VAntaNative.exe`, dependency license notices and `dist\VAnta-native-windows-x64.zip`. `-SkipTests` skips the test step for a previously verified source state.

```powershell
cd native
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo run -- --diagnostics ..\.tmp\native-diagnostics.json
cargo run -- --smoke-test
```

`--diagnostics` captures read-only real telemetry. `--inventory OUTPUT.json` exports read-only local service/startup inventory. `--smoke-test` visits all seven pages and closes after approximately eight seconds. Neither executes memory maintenance or GPU workloads. System-level maintenance validation requires a controlled test machine; normal smoke tests must not purge the user's memory or stop unrelated processes.

## Validation limits

Live process selection pins the full Windows creation FILETIME. Opened action handles must match that token, including changes within the same second. Telemetry still starts from a seconds-resolution identity: reuse within the same second before selection is a remaining observation limit. Historical records cannot authorize changes. Permission failures are reported, never assumed successful. The advanced system maintenance and complete feature parity status are documented in the release validation record.

See `RUST_RESEARCH.md` for primary-source architecture research and `NATIVE_VALIDATION.md` for measured local results. A language change by itself does not establish lower runtime memory, CPU use or faster startup.

## Administration limitations

Service polling has a ten-second transition deadline, but a synchronous Windows SCM call itself can block longer. The UI stays on a worker and shutdown waits for an in-flight native call. Services are Win32 services, not a full driver inventory.

Startup edits support current-user Run entries only; machine-wide and Startup-folder entries are observation-only. Native startup state uses Explorer's StartupApproved binary record. That format is undocumented and unsupported payloads are refused. The backend rechecks the command before writing, but registry command checking and approval writes are not an atomic transaction. The Python edition uses a different backup/remove method: do not switch between the two methods on the same entry without recapturing its state. Registry views and Windows versions need manual validation. No startup entries were modified during development checks.

System-cache trim uses the documented Windows trim sentinel. Standby purges use an undocumented native interface and have no cross-version compatibility guarantee. Required privileges are activated only on a temporary thread token, then discarded; the process token is not modified. Native tests cover the maintenance state flow with a fake adapter. No real global cache trim, standby purge, privilege activation or automatic elevation is part of the test suite.

Remaining Python parity work includes legacy SQLite/history import, long-term application usage, signed-in user aggregation, WDDM per-process GPU engines and memory, multi-adapter network/disk pages, and some Explorer/window actions. Native snapshots use a distinct format. FPS capture, hardware fault diagnosis and overclocking remain research topics, not delivered capabilities.
