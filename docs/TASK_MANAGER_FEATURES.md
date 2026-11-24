# VAnta Task Manager feature inventory

Date: 2026-11-15. Status: feature inventory; an initial implementation is available. See TASK_MANAGER_USAGE.md for the exact delivered scope and remaining gaps.

Scope: create a Task Manager module inside VAnta. This is a cross-version feature checklist; it is not certification that every item exists or is enabled in the locally inspected Taskmgr.exe build 10.0.26100.9818. Before claiming full parity, compare every page, selectable column, context menu, setting and permission case against a selected Windows build. Windows Insider announcements prove the announced preview capability, not universal stable availability.

## 1. Processes

- Apps, background processes and Windows processes; expand/collapse groups.
- Name, status, resource consumption and sortable columns.
- Per-process CPU, memory, disk, network, GPU and GPU engine when supported.
- Search/filter by executable name, PID or publisher.
- End task; restart Windows Explorer; switch to an application where applicable.
- Go to Details; open file location; file properties; online lookup.
- Efficiency mode for eligible processes; expose unavailable actions accurately.

Sources: S1, S2, S3, S4. Exact menu placement and optional columns require build-specific UI verification.

## 2. Performance

- CPU: usage graph, logical-processor graphs and kernel-time view.
- CPU detail checklist for UI verification: current/base speed, uptime, sockets, cores, logical processors, virtualization, cache and aggregate process/thread/handle counts.
- Memory: real-time use and composition. Detailed fields to verify: in-use/compressed, available, committed/limit, cached, paged/nonpaged pools, speed, slots, form factor and hardware reserved.
- Disk: activity graphs and per-device information. Detailed fields to verify: read/write rates, active time, response time, capacity, system/pagefile role and disk type.
- Network: per-adapter send/receive graphs; detailed addresses and link information to verify against the selected build.
- GPU: per-device engine utilization, dedicated/shared memory, GPU memory and supported device/driver details.
- GPU temperature where supported by hardware and driver.
- Compact graph/summary views, copy information and Resource Monitor entry point: verify exact options against the target build.
- NPU monitoring and per-process NPU columns: conditional. S8 documents the May 26, 2026 Windows 11 24H2/25H2 optional preview update; rollout state and supported hardware must be checked separately.

Sources: S1, S4, S5, S8, S9. Granular CPU/memory/disk/adapter field lists are parity acceptance candidates pending local UI verification, not newly established REA observations.

## 3. App history

- Historical CPU time and network use for the applications Windows tracks.
- Detailed checklist: metered/non-metered traffic, downloads/uploads and other optional columns; reset usage history; application switch action where offered.
- Do not label the view as complete historical activity for every executable.

Sources: S1 and S4; detailed column availability requires target-build verification.

## 4. Startup apps

- List registered startup applications, publisher, enabled state and startup impact.
- Enable/disable startup entries; file location, properties and online lookup.
- Last BIOS time where exposed; additional startup measurement columns are conditional acceptance candidates.
- Changing startup state is distinct from uninstalling or terminating an application.

Sources: S6, S4.

## 5. Users

- Signed-in users and resource consumption; expand a user's processes.
- User process actions; disconnect/sign-out operations and account-management entry points to verify by Windows edition, session type and privileges.

Sources: S1 and S4. Session-control menu availability must be verified locally.

## 6. Details and diagnostics

- Detailed process table and selectable columns.
- Candidate column families: identity/PID/user/session/status; CPU/time; memory variants; threads/handles; I/O counts/bytes; path/command line; platform/elevation/UAC; GPU usage/engine/memory; supported protection/architecture/power fields.
- End task and end process tree.
- Set process priority and processor affinity where supported.
- Analyze wait chain; create a process memory dump.
- UAC virtualization where applicable; open location; properties; online lookup; go to related services.
- System process live kernel dump: full live kernel dump or kernel stacks dump; configure dump options where supported.
- Exact column names and presence require target-build verification. A process dump records evidence for later analysis; it is not itself an automatic diagnosis.

Sources: S2, S4, S7. See the companion advanced-details research note in the investigation directory for additional official references.

## 7. Services

- Service name, PID, description, status and grouping information where displayed.
- Start, stop and restart services.
- Jump between a service and its process; open the Services management console.
- Service startup-type editing is a separate Services console capability, not baseline Task Manager behavior.

Sources: S1 and S4.

## 8. Shared controls and settings

- Run new task and supported administrator launch option (verify privilege behavior).
- Sort, select and resize columns; contextual navigation and copy actions where offered.
- Set default page, always on top and real-time update rate, including pause.
- Theme selection independent of Windows; Efficiency mode confirmation preference.
- Hide/minimize/tray behavior and keyboard navigation: verify version-specific controls.
- Show unsupported, unavailable and access-denied states; do not imply administrative actions always succeed.

Sources: S1, S3, S4 and S7.

## VAnta product additions tracked separately

The user also requests FPS monitoring, benchmarks, overclocking guidance and preliminary hardware symptom analysis. Keep these as separate product requirements instead of counting them as established Task Manager parity items. No implementation or final implementation sequence is approved by this inventory alone.

## Primary sources

S1. Microsoft Support, Use a screen reader to navigate Windows support tools, Task Manager section:
https://support.microsoft.com/en-us/accessibility/windows/use-a-screen-reader-to-navigate-windows-support-tools

S2. Microsoft Learn, Troubleshoot processes by using Task Manager:
https://learn.microsoft.com/en-us/troubleshoot/windows-server/support-tools/support-tools-task-manager

S3. Windows Insider Blog, Builds 22621.891 and 22623.891 (2022-11-10):
https://blogs.windows.com/windows-insider/2022/11/10/announcing-windows-11-insider-preview-build-22621-891-and-22623-891/

S4. Microsoft Press, Windows 10 Tools, chapter Mastering Task Manager (historical baseline; recheck current UI):
https://download.microsoft.com/download/7/3/8/7381E0E8-CE72-4366-9849-13B2BAFBBA3C/Microsoft_Press_ebook_Windows_10_Tools_Mobile-Only.pdf

S5. Microsoft DirectX Developer Blog, GPUs in the task manager:
https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/

S6. Microsoft Support, Configure Startup applications in Windows:
https://support.microsoft.com/en-us/windows/experience/startup-boot/configure-startup-applications-in-windows

S7. Microsoft Learn, Task Manager live memory dump:
https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/task-manager-live-dump

S8. Microsoft Support, KB5089573, May 26, 2026 (24H2/25H2 optional preview; NPU and Isolation columns):
https://support.microsoft.com/en-us/servicing/os/windows-11/2026/05/may-26-2026-kb5089573-os-builds-26200-8524-and-26100-8524-preview

S9. Windows Insider Blog, Build 18963, GPU temperature requirements:
https://blogs.windows.com/windows-insider/2019/08/16/announcing-windows-10-insider-preview-build-18963/

