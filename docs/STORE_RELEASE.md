# VAnta Store candidate — November 18, 2026

Status: locally verified candidate, not an installed or certified Store release. Application version 0.2.1, Windows x64. No Store submission, account change, certificate purchase, trust-store change or live system maintenance was performed.

## Product scope

The `store` Cargo feature uses separate VAntaStore data and provides read-only service/startup/system-memory administration. Backend calls reject service control, StartupApproved writes and system cache/standby operations. Startup enabled state is unknown. Resource observations, confirmed process actions, snapshots/CSV, GPU workload queue, CUDA pulse and Policy Lab remain available. No automatic elevation is requested. Custom workloads run with the user's permissions and are not security-sandboxed.

Windows workloads start suspended, enter a kill-on-close Job Object, and resume only after membership is verified and log readers are ready. Rust 1.100+ is required for the stable primary-thread handle. An OS file lock prevents simultaneous GUI sessions from overwriting the same queue and releases on exit. Optional NVIDIA libraries are loaded from Windows' system directory, not the current directory.

Built-in workloads persist a logical self-executable reference, resolved at dispatch to the current installation. A real subprocess regression deletes the old executable before recovering and running the queued workload through the new executable. Installed MSIX upgrade behavior still requires lifecycle testing.

## Build

Use Rust 1.100+ and Python 3.12+ from the repository root:

```powershell
.\scripts\build-native.ps1 -Store
python -m unittest discover -s scripts -p test_store_package.py -v
python scripts/store_package.py --preview --version 1.0.0.0 --makeappx 'C:\SDK\x64\MakeAppx.exe' --output dist/store-preview-new
python scripts/verify_store_payload.py dist/store-preview-new/VAnta.StorePreview_1.0.0.0_x64.msix .tmp/store-check-new
```

Use a new output directory for each package/evidence run. The packager preserves previous packages and staging directories. It rejects a portable binary, invalid/placeholder release identity, invalid version, non-x64 PE, missing native DLL dependency, missing notices or a linked payload path. On an MSVC host the Store build requests a static C runtime. Direct and delay imports are recorded; optional dynamic driver compatibility still requires hardware/OS testing.

## Current artifact and evidence

- `dist/store-candidate-2026-11-18-final/VAnta.StorePreview_1.0.0.0_x64.msix`: preview MSIX, 3,627,100 bytes. SHA-256: `10ec991ca8b4680eebacc1479820f5ef642923f92eac628ac26be1d918baa815`.
- Adjacent `.msix.json`: edition, identity, full file hashes, PE imports and checks not run. `makeappx.log`: successful SDK validation. MakeAppx's Microsoft Authenticode signature was Valid.
- `.tmp/store-release-2026-11-18/final-check/result.json`: exact extracted package payload tested without elevation. Passed help/version/release-info, real read-only diagnostics/inventory, invalid-argument rejection, seven-page GUI smoke, second-instance rejection and restart. Every startup row was non-writable/unknown. NVIDIA observations worked on this machine.
- Native tests: 71 portable and 67 Store passing; 10 ignored helper fixtures per edition are exercised by parent tests. Store omits portable-only administration mutation tests and adds backend-denial coverage. Both editions passed format/Clippy. Packaging contracts: 5 passing tests.
- Local 18.2-second observation: window visible at 0.227 s, mean working set 132.17 MiB, mean private bytes 79.16 MiB, normal close. One developer-machine observation is not a broad benchmark. Evidence: `.tmp/store-release-2026-11-18/measurement/result.json`. The Qt screenshot helper introduced padding; `overview-client.png` is a direct client-area capture. These are internal captures, not reviewed listing screenshots.
- CI covers both editions and preview packaging; no remote CI execution occurred in this session.

## Create the real Store package

Read `STORE_RELEASE_RESEARCH_2026-11-18.md` for official Microsoft sources. Copy the exact package name, publisher and publisher display name from the Partner Center product identity page. Do not invent these values or use Windows' special unsigned-development publisher OID for Store submission.

Run `scripts/store_package.py` with `--identity-name`, `--publisher`, `--publisher-display-name`, `--version`, `--makeappx` and a new `--output` directory; omit `--preview`. Release mode requires explicit identity values but cannot verify ownership/account validity without Partner Center. This package is unsigned for the documented Store-upload signing workflow. An unsigned upload package is not a generally installable sideload installer. No certificate purchase is required merely to prepare that MSIX upload.

The current manifest requests only runFullTrust, packagedClassicApp/mediumIL, x64 and a Windows.Desktop floor of build 19041. It does not request allowElevation. This declaration is not proof of testing on Windows 10. Application 0.2.1 and package 1.0.0.0 are separate version fields; choose consistent public release numbering before submission and increase package versions for upgrades.

## Remaining gates before submission

1. Supply real Partner Center identity and rebuild/validate that exact artifact. Explain runFullTrust accurately in certification notes. Identity validity and capability approval are account/reviewer decisions.
2. Supply publisher/support contact, publish the audited privacy policy at a public HTTPS URL, and enter that URL in the listing. `privacy.html` beside the candidate is ready-to-host local HTML, not a public URL. Replace the pre-release contact instruction before publication. Complete listing/screenshots/category/ratings/markets/pricing/license fields. `STORE_LISTING.md` is a draft.
3. Test the installed package on clean disposable Windows systems: install, normal launch, upgrade preserving settings/queue, uninstall and remaining files. Cover standard-user access denial, no NVIDIA, graphics/OpenGL, small monitors, 100/125/150/200% DPI, keyboard/screen reader, multiple displays and long sessions. Exercise only owned test child processes. Verify queued built-in pulse behavior after the package installation path changes. No clean-machine or installed-identity result is available here.
4. Run Windows App Certification Kit on the final identity/version in an active elevated test-machine session. WACK is absent from this host. MakeAppx validation and extracted EXE smoke are not substitutes. Follow Microsoft's documented command-line workflow and retain/review the complete report: https://learn.microsoft.com/en-us/windows/uwp/debug-test-perf/windows-app-certification-kit . A WACK pass does not guarantee Store certification.
5. Submit the reviewed artifact through Partner Center, retain ingestion/certification results and address any findings. No upload or publication was performed by this task.

## Preservation and limits

Original portable artifacts, Python code/data and prior uncommitted work are preserved. No commit, push, reset or cleanup was performed. Real CUDA load and live administration were not run. Initial process observations have seconds-resolution identity before the full FILETIME is pinned; same-second PID reuse before selection remains a limit. Synchronous native/driver calls can block worker shutdown. Missing Python parity is documented in NATIVE_USAGE.md; it is not claimed as delivered.

Primary-source Store research: `STORE_RELEASE_RESEARCH_2026-11-18.md`. Rust primary-thread handle: https://doc.rust-lang.org/std/os/windows/process/trait.ChildExt.html#tymethod.main_thread_handle .
