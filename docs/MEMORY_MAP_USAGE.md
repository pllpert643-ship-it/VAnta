# Memory Map ใน VAnta

## เป้าหมาย

Memory Map provides RAM diagnostics and confirmed manual maintenance. It is independently implemented and does not bundle or invoke RAMMap.

## Workflow ใช้งาน

1. เปิด VAnta แล้วเลือกแท็บ **Memory Map** ถัดจาก GPU Workloads / Task Manager
2. กด **Capture live** เพื่ออ่านข้อมูลเครื่องจริง งานอ่านข้อมูลทำใน worker thread แยกจากหน้าจอ
3. เปิด **Overview** ดู physical RAM, commit และ cache/kernel counters พร้อมแหล่งข้อมูลและข้อจำกัดของแต่ละค่า
4. เปิด **Processes** ค้นหาชื่อหรือ PID และเรียงคอลัมน์ working set/private bytes แบบตัวเลข
5. เลือกโปรเซสแล้วกด **Inspect selected process mappings** เพื่อดู mapping ของโปรเซสนั้น โดยตรวจ PID พร้อมเวลาเกิดโปรเซสอีกครั้งก่อนอ่าน
6. เลือกอัปเดตทุก 4/10 วินาที หรือ Pause การอัปเดตอัตโนมัติทำเฉพาะเมื่อ workspace นี้มองเห็น
7. กด **Save snapshot** เพื่อบันทึกข้อมูลรวมและรายการโปรเซสเป็น JSON ในตำแหน่งที่เลือก ไฟล์อาจมีชื่อโปรเซส/PID ควรระวังก่อนแชร์
8. กด **Open snapshot** เพื่ออ่านข้อมูลในอดีต การอัปเดตอัตโนมัติและการตรวจโปรเซสจาก PID ในไฟล์จะถูกปิด จนกด Capture live อีกครั้ง
9. อ่าน **Scope & definitions** เพื่อเข้าใจสิ่งที่วัดได้และสิ่งที่ยังไม่มี

## การตีความข้อมูล

- Available ไม่ใช่ Free page list และไม่ใช่พื้นที่ที่สูญเปล่า
- Working set คือหน้าหน่วยความจำ resident ซึ่งอาจแชร์กับโปรเซสอื่น การบวก working set ทุกโปรเซสจึงนับซ้ำได้
- Private bytes คือ private committed virtual memory ไม่ใช่ private resident RAM
- Windows GetPerformanceInfo SystemCache ครอบคลุม system working set และ standby pages ไม่ใช่ตัวเลข standby อย่างเดียว
- ข้อมูลหลายแถวอยู่คนละ accounting domain และทับซ้อนกัน ห้ามรวมทุกแถวเป็นยอด RAM เดียว
- Process mappings เป็นการดู address-space mapping ที่ระบบเปิดเผย ไม่ใช่ PFN inventory, global File Summary หรือ File Details แบบ RAMMap
- Unavailable หมายถึงวัดไม่ได้/สิทธิ์ไม่พอ/ไม่รองรับ ไม่ใช่ศูนย์ และไม่ใช่การวินิจฉัยว่าฮาร์ดแวร์เสีย

## ขอบเขตด้านความปลอดภัย

Maintenance now includes selected-process trim, system file-cache trim, and advanced standby purge. There is no automatic clearing, all-process trim, modified-list write, driver loading, or automatic elevation. Each action requires a live capture and an explicit confirmation. Snapshot export uses atomic replacement to preserve the previous file on write failure.

ยังไม่มี RAMMap PFN Use Counts, exact page-state lists, Priority Summary, Physical Pages/Ranges, global File Summary/Details และการเปิด/บันทึก .RMP ไม่สร้างค่าจำลองมาเติมช่องว่างเหล่านี้

## โครงสร้าง

```text
MemoryMapWidget (UI + historical/live state)
        ↓ queued signals
MemoryWorker (dedicated QThread)
        ↓
MemoryMapCollector (read-only metrics/processes/mappings)
        ↓
psutil + Windows GetPerformanceInfo
```

Snapshot เป็นข้อมูล DTO แยกจาก Qt และตรวจ schema/type/range ตอนนำเข้า ประวัติ Task Manager และกลไกคิว GPU เดิมไม่ถูกแทนที่

## ตรวจสอบ

```powershell
.\.venv\Scripts\python.exe -m pytest -q
```

รายงาน REA และขอบเขตหลักฐาน: `docs/RAMMAP_RESEARCH.md` และ `reference/rammap-evidence/`


## Prior diagnostic release validation - 2026-11-16

- Live worker and full MainWindow integration: three workspaces present, real memory/process observations populated, shutdown completed without a running worker.
- Reviewed a real MainWindow screenshot for styling and readable counters.
- Reviewer findings fixed: stop GPU admission immediately on exit; suspend automatic collection during snapshot dialogs; retain explicitly timestamped mapping captures across ordinary refreshes.
- Rebuilt `dist/VAnta/VAnta.exe`; packaged `--help` and GUI launch/normal-close smoke passed using an isolated temporary LOCALAPPDATA.
- Updated `dist/VAnta-windows-x64.zip`, verified executable entry and ZIP CRCs. Existing Desktop shortcut already targets the rebuilt executable.
- psutil process-mapping reads are synchronous native calls with no safe cancellation/timeout. Worker collection keeps the UI responsive, but exit can wait for an in-flight read. No forced thread termination is used.
- RAMMap itself was inspected statically only, not launched or compared at runtime; this is not full RAMMap parity certification.

## Memory maintenance workflow - 2026-11-16

1. Open **Memory Map** and click **Capture live**. Historical JSON snapshots cannot authorize changes to live processes or system memory.
2. For a single application, open **Processes**, select its row, and choose **Trim selected process working set**. Windows verifies the opened process creation time before changing its working set.
3. For system-wide actions, launch VAnta explicitly with **Run as administrator**. The app does not automatically elevate. Administrator membership alone is insufficient if the process token lacks the required privilege.
4. Choose an operation, then press **Review and run...**. Read the target and effects. **Cancel** is the default and sends no command. Refresh pauses while the confirmation dialog is open.
5. Confirm once. The command runs in the worker thread with no overlapping operation. Read the result and available-RAM observations; a fresh live capture follows a successful command.
6. If Windows denies access or does not support the operation, the UI reports failure instead of assuming success. Do not interpret before/after available RAM as an attributable amount freed.

| Operation | Effect and limits |
| --- | --- |
| Trim selected process | Calls documented EmptyWorkingSet for one verified process; pages can be faulted back in. Private committed memory is not deallocated. Protected or inaccessible targets can fail. |
| Trim system file-cache working set | Calls documented SetSystemFileCacheSize with the trim sentinel; requires SeIncreaseQuotaPrivilege. This is not a full standby purge. |
| Purge standby list (advanced) | Uses NtSetSystemInformation / SystemMemoryListInformation. Undocumented interface; compatibility is not guaranteed across Windows versions. Requires administrator and SeProfileSingleProcessPrivilege. |
| Purge priority-0 standby (advanced) | Same native-interface caveat; targets the priority-0 standby command. |

Cached RAM is normally reusable by Windows. Clearing it can slow workloads through extra disk reads and page faults. These commands do not fix leaks, delete user files, or guarantee higher performance. No command is run on a schedule.

## Release validation limits

Automated tests use a fake native adapter for system-wide changes. They must never purge standby, trim the system cache, or trim unrelated applications on the developer machine. Native process-trim verification may target only an owned test child. Actual system-wide behavior, UAC/privilege combinations, Windows build compatibility, and performance effects require a controlled manual validation machine. Passing the test suite does not certify full production readiness or RAMMap parity.

The existing synchronous process-mapping reader still cannot be safely interrupted. Application exit waits for an in-flight worker call; it never forcibly terminates the thread.

## Maintenance build validation - 2026-11-16

- Full regression suite: **136 passed in 21.00s** (38 maintenance backend tests, 63 snapshot/collector tests, 11 Memory Map UI tests, plus existing regressions).
- Read-only native smoke: real process handle creation time matched psutil; elevation detection and available-memory sampling returned successfully.
- Real EmptyWorkingSet smoke: executed once against an owned 16 MiB test child only; Windows returned success and the child exited normally. The venv launcher/child relationship was verified before choosing the target PID. No unrelated application was trimmed.
- No system-wide cache trim, standby purge, privilege activation, or elevation was performed. Those effects remain unvalidated on a controlled test machine.
- Full MainWindow smoke: live data populated for 304 processes, all four maintenance choices present, screenshot inspected, normal shutdown succeeded.
- Review fix covered by regression: a maintenance error can occur after memory changed; both success and failure now invalidate old mappings and request a fresh capture. The operation result remains visible through ordinary refresh.
- PyInstaller build completed; archive table contains vanta.infra.memory_actions. Packaged GUI launch and normal WM_CLOSE passed with isolated LOCALAPPDATA; 126 application observations were persisted.
- Updated dist/VAnta-windows-x64.zip and verified its CRCs. The Desktop shortcut continues to target dist/VAnta/VAnta.exe.

Before calling the advanced system-wide commands production validated, execute the manual matrix on disposable Windows test machines: supported Windows builds, standard vs elevated token, missing privilege, successful system-cache/standby operations, working-set and disk-I/O effects, and repeated launch/exit. Unsupported native interfaces cannot carry a Microsoft compatibility guarantee.
