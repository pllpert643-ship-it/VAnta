# VAnta native Rust migration research

Date: 2026-11-17. Scope: read-only inspection of the Python application and official upstream documentation; architecture advice for the parallel Rust implementation. This note is research, not certification of the Rust build. Only this file was authored by the research worker.

## Recommendation and evidence status

**Use eframe/egui for the first native Rust release**, with background collection, a deterministic scheduler, native Windows operations behind a small interface, and SQLite compatibility. This is an engineering recommendation based on the application's dense tables, telemetry graphs, and existing Python/Qt implementation; it is not a measured performance ranking. Eframe provides a native Rust application path and selectable graphics renderers [S1, S2].

**Measured in this investigation:** no executable size, RSS/private bytes, CPU/GPU utilization, startup latency, frame time, battery use, or accessibility conformance. No app, benchmark, process-changing command, elevation, or memory maintenance operation was run. Existing validation statements in other documents were not independently reproduced. Source inspection establishes intended/implemented code paths, not successful runtime behavior. A smaller footprint than Python/PySide/PyInstaller is a plausible hypothesis that must be tested.

Links below point to official projects or vendors. Online `latest` pages and repository default branches move: select a mutually compatible toolchain/dependency set, commit Cargo.lock, and verify documentation against the resolved versions. This note intentionally does not prescribe floating `*` versions.

## Existing behavior to preserve

Local evidence is the inspected source and the delivered-scope documents, not the larger feature wish list in TASK_MANAGER_FEATURES.md:

| Workspace | Observed source behavior / preservation requirement | Local evidence |
| --- | --- | --- |
| GPU Workloads | Strict priority then FIFO; one concurrent job; 512 MiB reserve (current names say MB, arithmetic uses 1024 squared); queued/waiting/running/terminal states; custom argument-vector commands, cancellation and logs; persisted jobs/events, interrupted-process recovery; demo and deterministic Policy Lab | [rules](../src/vanta/core/rules.py), [scheduler](../src/vanta/core/scheduler.py), [runner](../src/vanta/infra/processes.py), [models](../src/vanta/core/models.py), [storage](../src/vanta/core/storage.py), [Policy Lab](../src/vanta/core/policy_lab.py) |
| Task Manager | Process search/sort/columns, resource histories, details/actions, performance/hardware, locally observed app history, startup backup/restore, sessions, services, settings and new-task launch | [delivered scope](TASK_MANAGER_USAGE.md), [collector/actions](../src/vanta/infra/task_manager.py), [Windows operations](../src/vanta/infra/windows_tasks.py), [startup](../src/vanta/infra/startup.py) |
| Memory Map | Physical/commit/cache/pool observations; process working set/private bytes; opt-in grouped mappings; validated JSON save/load with atomic replacement; live/historical distinction; confirmed selected-process trim, system-cache trim, advanced full/priority-0 standby purge | [collector/schema](../src/vanta/infra/memory_map.py), [maintenance](../src/vanta/infra/memory_actions.py), [usage and limits](MEMORY_MAP_USAGE.md) |

Specific migration traps found in source:

- When GPU telemetry is unavailable, `RuleEngine.evaluate` currently **permits launch subject to concurrency alone**. Preserve this explicitly or introduce a visible, documented policy setting with a migration decision. Do not accidentally claim VRAM admission remains enforced without measurements.
- A waiting highest-priority job blocks smaller lower-priority jobs. Fit-what-fits is a Policy Lab alternative, not the live default.
- The two-second post-launch "Verified" event records a global before/after VRAM delta. It does not establish allocation ownership, successful computation, or absence of other GPU users. Preserve the observation with accurate wording.
- The built-in pulse is real CUDA work: the existing [workload](../src/vanta/workloads/gpu_pulse.py) loads `nvcuda.dll`, allocates device memory and repeatedly fills/synchronizes it. Preserve an internal `--gpu-workload` child mode in the Rust executable using dynamically loaded CUDA Driver functions with RAII cleanup [S21]; a CPU busy loop is not a substitute for this feature. Do not leave the packaged app dependent on a Python interpreter.
- SQLite stores command arguments as JSON, timestamps as strings, and before-snapshots as JSON. Preserve data paths and identifiers, test old fixtures, and version schema changes. Do not silently discard history or open two writers with incompatible migrations.
- Task Manager app history is VAnta's observed CPU/I/O delta history, not Windows SRUM history. Startup coverage is Run keys and Startup folders, not every Windows startup mechanism. Its grouping is heuristic, not Windows Task Manager grouping [local delivered scope].
- Memory snapshots deliberately leave exact PFN page states unavailable. Windows psutil mapping `rss` is suppressed because the existing reader's field is not measured residency. Preserve this semantic distinction in new region displays [local collector].

## Desktop framework comparison

The ordering is suitability for this migration, not a resource benchmark. None of these choices alone guarantees Windows-native controls or accessibility conformance.

| Rank / option | Documented capabilities | Fit and limitations for VAnta | License/distribution |
| --- | --- | --- | --- |
| 1. eframe/egui | Rust immediate-mode UI; native Windows application integration; graphics renderer selection; optional AccessKit enabled by default in eframe [S1, S2] | Direct fit for frequently changing tables and charts. Own selection/sort/focus state explicitly, virtualize large tables, and provide semantics for custom drawing. Upstream explicitly does not aim for native-looking widgets; updates can break interfaces [S1]. | MIT OR Apache-2.0 [S1]. No WebView UI required by this approach. Retain dependency and font notices; eframe has substantial dependencies despite egui's small core [S1]. |
| 2. Slint | Declarative `.slint` UI compiled into Rust; property bindings and transitions; built-in property animation [S5, S6] | Strong alternative when a designed animated UI is the priority. Adds a markup/build step and model bridging. Validate large dynamic tables, screen-reader behavior, and chosen renderer on Windows before selection; no tested superiority claimed. | Framework offers GPLv3, royalty-free desktop/mobile/web terms, or commercial terms [S4]. Royalty-free is not MIT: attribution and use restrictions apply [S4a]. |
| 3. Tauri | Rust application with web frontend; Windows installer supports WebView2 deployment choices [S3, S3a] | Feasible if maintaining web UI expertise is the priority. Adds a web frontend and command/serialization seam to native monitoring. Measure all WebView-related processes when comparing memory; small application download does not prove small runtime footprint. | MIT OR Apache-2.0 for Tauri [S3a]; Windows WebView2 is a separate runtime. Bootstrapper/offline/fixed runtime choices affect installation size and updates [S3]. |

**Renderer decision:** start with one eframe renderer, retain AccessKit, and avoid enabling both graphics stacks without a tested fallback need. Wgpu is a reasonable Windows candidate; glow is worth a size/compatibility experiment. Eframe documents both and notes that glow can reduce binary size, but that is upstream guidance, not a VAnta measurement [S2]. Ensure UI GPU activity is counted in VAnta's own monitoring overhead.

## Telemetry and Windows operations

| Module implementation | Recommended role | Semantics and constraints |
| --- | --- | --- |
| `sysinfo` | Persistent process/system collector for CPU, RAM, process identity and supported I/O/network/disk observations | Keep one System instance and refresh only needed fields. Rate metrics need successive samples and sufficient interval; first sample is warming up. OS support does not imply every field is supported on Windows [S7]. Normalize per-process CPU to a documented total-machine or single-core scale. |
| `nvml-wrapper` | Optional NVIDIA adapter for device utilization, memory, temperature and supported power readings | Initialize once; dynamically load NVML; handle missing DLL/functions and unsupported fields individually [S8]. NVML readings depend on driver/platform; Windows WDDM memory accounting differs from TCC. Do not equate global device memory deltas to per-process allocations [S9]. |
| `windows` crate | Typed Windows calls for PDH, process handles/times, memory, sessions, services, startup registry, shell integration and documented process actions | Microsoft maintains Rust Windows bindings [S10]. Enable only feature namespaces actually used. Confine unsafe calls, handle ownership, buffer checks, privilege restoration and error conversion inside this implementation. Bindings do not make a native operation permission-free. |
| Windows PDH GPU counters | Preserve engine, per-process dedicated/shared and adapter memory observations across supported GPU vendors | Existing code uses English PDH counter paths and Windows LUID matching. Preserve that mapping and counter status checks. Microsoft's Task Manager obtains WDDM data from VidSch/VidMm and uses busiest-engine utilization; do not blindly sum engine percentages or duplicate process memory [S11; local GpuCounters]. |

Store a value with its unit, source, capture time, validity and error reason; missing/denied/stale is not zero. Keep NVML and Windows engine measurements separately labeled. Process network throughput, game FPS, NPU support, disk response time, and hardware health diagnosis are not supplied simply by adding these crates; each requires its own collector, evidence and acceptance work. This is a scope recommendation, not a claim of a new implementation.

For Memory Map, use documented GetPerformanceInfo aggregates: multiply page counters by PageSize; PhysicalAvailable includes standby/free/zero lists, and SystemCache includes standby plus system working set. Neither is an exact standalone standby measurement [S12]. Use VirtualQueryEx for virtual regions; optionally query selected page addresses with QueryWorkingSetEx for actual per-page observations [S13, S14]. Region size alone is not resident bytes. Bound output and address arithmetic, hold a verified process handle, and timestamp partial captures.

For maintenance, preserve EmptyWorkingSet's documented rights and explicit confirmation; preserve SetSystemFileCacheSize's sentinel and SeIncreaseQuotaPrivilege requirement [S15, S16]. Retain advanced standby purge only in an isolated, explicitly unsupported Windows-native implementation with capability/error reporting and manual controlled-machine validation. The existing `NtSetSystemInformation` information-class usage is source-observed, not a Microsoft-supported contract established by this research. Never infer bytes "freed" causally from changing global available memory [local maintenance/usage].

## Concrete architecture and interfaces

This is a proposed organization, not instructions to overwrite the parallel implementation. Prefer deep modules: callers learn a small interface while native complexity and verification remain local.

```text
Native UI (GPU Workloads | Task Manager | Memory Map)
    -> typed commands / operation receipts
    <- latest immutable snapshot + bounded event history
Background application coordinator
    |-- scheduler: deterministic decisions + explicit effects
    |-- observation: sysinfo + NVML + Windows PDH/PSAPI
    |-- Windows operations: identity checks + owned handles + privileges
    |-- persistence: existing SQLite data + validated snapshot JSON
    `-- workload runner: child ownership, logs, exit/recovery
```

- **Observation module:** `refresh(request) -> Snapshot`, with cheap routine sampling separate from requested mappings/hardware inventory. Production and recorded-fixture adapters provide a real test seam. Coalesce stale observations into a latest-value slot; bound histories and pending commands. A slow sample must not accumulate an unlimited queue.
- **Scheduler module:** `advance(state, observation, commands, now) -> decisions/effects`. It owns priority ordering, head-of-line blocking, admission explanations and terminal transitions. The runner executes effects and returns outcomes. Tests exercise the same interface with fake time and recorded observations.
- **Windows operations module:** `execute(confirmed_command) -> OperationReceipt`. A command carries `(pid, creation_time)` where applicable, live-context identity, target, operation and request ID. Open once, verify identity on that handle, then act on that handle. Confirmation is user intent, not authorization inferred from a historical row. Serialize conflicting operations and return native errors rather than optimistic success.
- **Persistence module:** owns migrations, transactions, atomic snapshot writes, type/range/schema validation and data location. Tests use actual temporary SQLite databases and old-format JSON fixtures. Imported historical files cannot authorize live operations.
- **Workload runner module:** own children and handles, argument-vector launch, per-job logs, graceful exit/cancellation then explicit termination policy. Windows Job Objects can manage associated process trees [S17]. A future kill-on-close implementation must handle assignment failure, nested jobs and escaped descendants; never claim crash cleanup just because a root PID was stored.

One coordinator thread plus a dedicated bounded expensive-inspection worker is enough initially; introduce an async runtime only if justified. UI consumes snapshots without blocking or holding collection locks while painting. Shutdown stops admission first, requests cancellation, joins workers and closes native resources. Cooperative cancellation checks between native calls cannot guarantee interruption of an already blocked call. For a strict shutdown deadline, isolate potentially blocking inspection in an owned helper process and test its lifecycle.

**Parity gate:** compare each row/action/settings flow in the local delivered-scope documents against the Rust app. A three-tab shell with aggregate statistics is not preservation of Task Manager, Memory Map or scheduling. Record each feature as ported/validated, explicitly deferred, or unavailable with reason. Port deterministic scheduler tests, PID-reuse checks, historical/live restrictions, denied-access paths, startup restore conflicts and malformed snapshot tests. Real mutation integration tests target owned children; system-wide operations require a controlled test machine [local tests and usage limits].

## Animation, reduced motion and accessibility

Implement short, interruptible selection/hover fades and workspace transitions; suggested durations of 100–180 ms are design choices, not measured usability results. Animate presentation, never admission decisions, selection identity or recorded telemetry. Egui supplies time-based animation and explicit repaint requests [S18]; Slint supplies declarative property animation [S6]. Repaint during active transitions, input and fresh observations rather than running a permanent animation loop.

Read `SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION)` or `UISettings.AnimationsEnabled` and offer **Follow system / Reduce motion** [S19, S20]. On disabled animations, snap to final states, remove moving background effects and stop decorative pulse/spinner animation; use static progress text. Re-read on settings changes or application activation. Do not assume the framework automatically applies this Windows preference.

Keep AccessKit enabled in a stripped-down eframe configuration [S2]. Use labeled standard controls, stable IDs/focus, keyboard operation for tables/context actions, readable text equivalents for graphs, non-color status indicators, DPI scaling and a high-contrast design. Test with Windows Narrator, keyboard-only navigation, 125–200% scaling, and runtime reduced-motion changes. AccessKit support is infrastructure, not evidence that VAnta's custom charts/tables or dialogs are accessible. Explicitly test Cancel default focus and focus restoration after operation results [S1; proposed acceptance criteria].

## Licensing and ranked enhancements

The current repository LICENSE is MIT. Egui/eframe, Tauri and nvml-wrapper offer MIT OR Apache-2.0; sysinfo is MIT; Microsoft windows-rs includes MIT terms [S1, S3a, S8a, S7a, S10a]. Slint framework licensing is distinct from its MIT documentation/examples. Its royalty-free 2.0 route requires an accessible AboutSlint widget or public attribution badge, excludes embedded usage, and restricts standalone framework distribution/exposure [S4, S4a]. Review terms for the exact resolved release. Produce third-party notices including fonts/assets and transitive dependencies; loading an installed NVIDIA runtime does not grant rights to redistribute NVIDIA binaries under a Rust wrapper's license.

These are ranked engineering/product recommendations by expected user value and feasibility, not measured benefit estimates:

1. **Capability and freshness indicators:** show missing driver, denied permission, warming-up and stale data next to the affected field. High value, small scope; prevents false readings across all workspaces.
2. **Responsive large process tables:** visible-row rendering, numeric sorting, persistent selection by process identity, pause/search and detail-only expensive queries. Validate with recorded large snapshots and live refresh.
3. **Scheduler explainability and ownership:** show exact VRAM arithmetic and telemetry policy; distinguish observation from job verification; expose cancellation/recovery outcomes. Add tested Job Object ownership where feasible [S17].
4. **Memory snapshot comparison:** compare timestamped captures using creation-time identity, explicit accounting definitions, and nullable deltas. Useful diagnostics without speculative automatic cleanup.
5. **Low-overhead collection controls:** preserve pause/update rates; suspend hidden expensive views, retain bounded history, and expose VAnta's own CPU/private memory. Measure foreground/minimized behavior before making efficiency claims.
6. **Accessibility/settings polish:** reduced motion, keyboard/focus, contrast, scalable fonts and readable graph alternatives. Treat basic keyboard and labels as release requirements, even though richer settings can follow.
7. **Multi-GPU selection and correlation:** stable device identity and explicit NVML-to-Windows adapter correlation. Do not confuse NVML index 0 with Windows GPU 0 [S11].
8. **Later opt-in investigations:** ETW/PresentMon-based FPS, reproducible benchmarks, and symptom evidence reports. Each is a separate research/validation effort; avoid unsupported overclock automation or hardware-failure conclusions from temperatures alone.

To evaluate "lightweight," build release artifacts and record the exact commit, Cargo.lock, Windows build, hardware/driver, renderer and sampling interval. Compare the current Python build with the Rust candidate using the same workload: cold/warm startup, 60 seconds idle foreground, minimized, active process table, mapping inspection and job launch/cancel. Record whole-process-tree private bytes/working set, CPU time, GPU utilization, executable plus dependencies, input latency and shutdown latency. Repeat runs and report variability; keep engineering targets separate from observed results.

## Primary source links

- S1. [Egui upstream capabilities, accessibility and licenses](https://github.com/emilk/egui).
- S2. [Eframe native integration and feature flags](https://docs.rs/eframe/latest/eframe/).
- S3. [Tauri Windows installer and WebView2 deployment](https://v2.tauri.app/distribute/windows-installer/).
- S3a. [Tauri upstream architecture description and licenses](https://github.com/tauri-apps/tauri).
- S4. [Slint framework license selection](https://raw.githubusercontent.com/slint-ui/slint/master/LICENSE.md).
- S4a. [Slint royalty-free license 2.0 terms](https://raw.githubusercontent.com/slint-ui/slint/master/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md).
- S5. [Slint Rust integration and compilation](https://docs.rs/slint/latest/slint/).
- S6. [Slint property animation](https://docs.slint.dev/latest/docs/slint/guide/language/coding/animation/).
- S7. [Sysinfo usage, refresh and sampling guidance](https://docs.rs/sysinfo/latest/sysinfo/).
- S7a. [Sysinfo MIT license](https://raw.githubusercontent.com/GuillaumeGomez/sysinfo/main/LICENSE).
- S8. [Nvml-wrapper initialization and dynamic loading](https://docs.rs/nvml-wrapper/latest/nvml_wrapper/).
- S8a. [Nvml-wrapper upstream licenses](https://github.com/rust-nvml/nvml-wrapper).
- S9. [NVIDIA NVML device queries, memory semantics and unsupported errors](https://docs.nvidia.com/deploy/nvml-api/latest/api/group__nvmlDeviceQueries.html).
- S10. [Microsoft Rust for Windows](https://github.com/microsoft/windows-rs).
- S10a. [Microsoft windows-rs MIT license](https://raw.githubusercontent.com/microsoft/windows-rs/master/license-mit).
- S11. [Microsoft: GPUs in Task Manager](https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/). Historical design explanation; not current-build parity certification.
- S12. [Microsoft PERFORMANCE_INFORMATION field definitions](https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-performance_information).
- S13. [Microsoft VirtualQueryEx](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualqueryex).
- S14. [Microsoft QueryWorkingSetEx](https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-queryworkingsetex).
- S15. [Microsoft EmptyWorkingSet](https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-emptyworkingset).
- S16. [Microsoft SetSystemFileCacheSize](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-setsystemfilecachesize).
- S17. [Microsoft Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects).
- S18. [Egui Context animation and repaint methods](https://docs.rs/egui/latest/egui/struct.Context.html).
- S19. [Microsoft SystemParametersInfoW; SPI_GETCLIENTAREAANIMATION](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfow).
- S20. [Microsoft UISettings.AnimationsEnabled](https://learn.microsoft.com/en-us/uwp/api/windows.ui.viewmanagement.uisettings.animationsenabled).

- S21. [NVIDIA CUDA Driver memory allocation, fills and cleanup](https://docs.nvidia.com/cuda/cuda-driver-api/group__CUDA__MEM.html).

