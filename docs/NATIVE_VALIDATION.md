# VAnta Native validation

Date: 2026-11-17. Local Windows x64 development build. This record distinguishes implemented behavior, tests and controlled-machine work that remains. It does not certify full Python/Task Manager/RAMMap parity or production readiness.

## Delivered architecture

Rust native application in `native/`, eframe/egui with a single OpenGL renderer, sysinfo background observations, dynamically loaded NVIDIA NVML/CUDA, Windows process/memory/service operations, local atomic versioned JSON storage and isolated data directory. Python source, packaged executable, existing shortcut and SQLite data are preserved.

Research is in `RUST_RESEARCH.md`. Implementation choices use official crates and Windows interfaces; no third-party application was cloned or bundled. Dependency and font licenses are included in the portable package.

## Automated evidence

- Native behavior suite: 67 passing tests, 9 ignored subprocess fixtures invoked by the behavior tests; no failing tests at the recorded checkpoint.
- Coverage includes strict/FIFO admission, reserve rejection/waiting, pause/recovery, literal argv, owned-child cleanup, post-assignment descendant cleanup and controller-crash behavior, post-launch global VRAM observation, bounded logs/history, corrupt-storage preservation, atomic snapshots and input validation, CSV escaping, telemetry intervals, service/startup input validation, confirmed administration worker behavior, and privilege/error/cleanup sequencing through a fake maintenance adapter.
- Native process mutation tests target owned helper children only. Full creation-token mismatch (including within one Unix second), unpinned mutations and protected PIDs are rejected.
- GUI smoke visits overview, processes, memory, policy, workloads, administration and settings, then exits normally. It does not capture/change services or startup or run maintenance automatically.
- Read-only diagnostics captured 315 processes, 12 logical CPUs and approximately 15.71 GiB physical RAM. Native service/startup inventory returned 296 services and 16 startup entries with an unelevated token. Counts describe one capture and vary as the system changes.

## Initial measurement, before final UI tuning

One local sequential observation per edition, roughly 18 seconds after the window appeared; both used isolated local data. Native observed its default overview at a two-second capture interval. Python used its existing default GPU-workload overview and its own polling configuration. These are different workloads; the comparison cannot isolate language, and is not a controlled performance benchmark.

| Observation | Python packaged edition | Initial Rust release build |
| --- | ---: | ---: |
| Launch to visible window | 5.150 s | 0.444 s |
| Mean process-tree working set | 133.84 MiB | 144.23 MiB |
| Peak sampled working set | 205.18 MiB | 146.51 MiB |
| Mean process-tree private bytes | 96.62 MiB | 79.62 MiB |
| CPU seconds over approximately 18 s | 13.734 | 2.156 |
| CPU as percentage of one logical core | 76.30% | 11.98% |
| Normal window close | passed | passed |

Working set and private bytes measure different things. The initial Rust working set was higher on average, so this evidence does not establish that Rust is uniformly lighter. CPU percentages here use one-core scale, unlike the total-machine percentage in the app. No claim is made about reduced GPU overhead or hardware compatibility beyond this machine.

## Remaining release validation

- Run multiple matched workloads and repeated launches with both versions, equal sampling intervals and comparable pages; include idle, process lists, graphs, minimized windows, long sessions and limited hardware. Record process-tree RAM/private bytes, CPU, GPU overhead, startup and package size independently.
- Validate documented service controls and startup enable/disable on disposable entries/test services, including access denial, state changes, malformed records, registry views and supported Windows versions.
- Validate system-cache and advanced standby commands only on controlled disposable Windows test machines, including normal/elevated tokens, missing privileges, failure after a native side effect, repeated operations and disk/page-fault effects. No global maintenance has been performed on the user's working desktop.
- Validate complete keyboard and screen-reader workflows. AccessKit is enabled and values have textual labels, but this is not accessibility certification.
- Windows Job Objects start containing the child only after std::process spawning and assignment; a pre-assignment descendant race remains. Tests verify contained descendants rather than proving no escape window.
- Native synchronous calls can delay shutdown; no worker thread is forcibly terminated. Service transitions are time-bounded after native calls return.
- Complete the remaining migration items listed in `NATIVE_USAGE.md` before replacing the Python edition or claiming complete feature parity.

## Final portable release checkpoint

Packaged version: 0.2.0, Windows x64, November 17, 2026. `cargo fmt --check`, `cargo clippy --locked --all-targets -j 4 -- -D warnings` and the 67-test native suite passed. The original Python suite also passed: 136 tests. Nine ignored native items are subprocess fixtures exercised by behavior tests.

The packaged executable passed `--help`, `--version`, read-only `--diagnostics`, read-only `--inventory`, and `--smoke-test`, all exit code zero with empty stderr. The release GUI smoke visited seven workspaces and closed normally in 8.407 seconds. Final diagnostics captured 319 processes. ZIP CRC verification passed for all 556 members, including dependency notices and the font OFL license. CI configuration is supplied but has not been run remotely.

| Observation | Python packaged edition | Final Rust portable edition |
| --- | ---: | ---: |
| Launch to visible window | 5.150 s | 0.350 s |
| Mean process-tree working set | 133.84 MiB | 134.33 MiB |
| Peak sampled working set | 205.18 MiB | 136.26 MiB |
| Mean process-tree private bytes | 96.62 MiB | 79.06 MiB |
| CPU seconds during observation | 13.734 | 2.000 |
| CPU percentage, one-core scale | 76.30% | 10.94% |
| Portable folder | 121,538,590 bytes | 7,576,434 bytes |
| ZIP | 49,237,997 bytes | 3,532,205 bytes |
| Normal window close | passed | passed |

The same workload and repeatability caveats from the initial observation apply. Final native observation lasted 18.275 seconds; the harness now uses measured elapsed time for CPU percentage. These observations suggest faster startup, less CPU time and smaller distribution on this machine, while mean working set remained approximately equal. They do not establish a language-only benefit or general performance guarantee. The final overview screenshot was visually reviewed.

Artifacts:

- `dist/VAnta-Native/VAntaNative.exe`: 4,794,368 bytes; SHA-256 `39c067b03028969a9d288c0f828b3c7cf80a34ebe4f47b447230f56cde10c96e`.
- `dist/VAnta-native-windows-x64.zip`: 3,532,205 bytes; SHA-256 `3684521105821e8f3250c487e67f593d1eb79828127638d7535eea102018ae83`.
- Raw local evidence: `.tmp/native-final/result.json`, `.tmp/native-final/overview.png`, `.tmp/release-validation/results.json`.

No real CUDA pulse, service control, startup mutation or global memory-maintenance command was executed in this validation. Their implementation must be distinguished from integration validation on a disposable machine.


Final review notes: maintenance freshness is rechecked when the confirmation is accepted, with a regression test for expiration while the dialog remains open. Native JSON snapshots capture telemetry fields only; separately collected Windows commit/cache/kernel-pool counters and virtual-region tables are not included in the saved document. Historical mode hides those live-only observations. The telemetry polling deadline is not a hard worker shutdown deadline when an underlying native call blocks.

## Store preparation checkpoint — November 18, 2026

Application source version 0.2.1. This checkpoint supersedes the historical pre-assignment race limitation above: workload launch now uses CREATE_SUSPENDED, assigns the child to the kill-on-close Job Object, prepares log readers, verifies membership, then resumes the primary thread. A real suspended-child regression passed. This does not establish clean-machine package compatibility.

Both editions passed formatting and Clippy with warnings denied. Full native suites passed: Store 67, portable 71, with 10 ignored subprocess fixtures exercised through parent behavior tests per edition. Coverage includes exclusive GUI data-directory locking, absolute system-directory vendor DLL paths, Store backend mutation denial and built-in workload recovery after deleting the old installation executable. Five packaging contract tests passed.

Final preview artifact: `dist/store-candidate-2026-11-18-final/VAnta.StorePreview_1.0.0.0_x64.msix`, 3,627,100 bytes, SHA-256 `10ec991ca8b4680eebacc1479820f5ef642923f92eac628ac26be1d918baa815`. MakeAppx validation and ZIP CRC passed. The exact extracted payload passed CLI/version/edition checks, diagnostics, read-only inventory, invalid-argument rejection, seven-page GUI smoke, simultaneous-instance rejection and restart after exit. Host token was not elevated; NVIDIA observations were available. Evidence: `.tmp/store-release-2026-11-18/final-check/result.json`.

This tested the extracted payload, not a Windows-installed MSIX identity. WACK, actual installation/upgrade/uninstall, clean Windows/no-NVIDIA compatibility, full accessibility and long-session validation remain outstanding. No Store upload/publication or real CUDA/system-administration workload was performed. See STORE_RELEASE.md for remaining gates and STORE_LISTING.md for the draft listing.
