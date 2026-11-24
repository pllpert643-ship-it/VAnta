# VAnta Microsoft Store listing draft

Prepared November 18, 2026 for the native Store edition, application 0.2.1. Draft only; no listing has been uploaded. Screenshots and publisher details must describe the exact final Store build. The application interface is English; the Thai listing below does not imply a Thai interface.

## English listing

**Product name:** VAnta (subject to Partner Center name availability)

**Short description:** Inspect PC resources and manage a visible queue of NVIDIA GPU workloads.

**Description:**

VAnta brings local CPU, RAM, process and supported NVIDIA GPU observations into one Windows desktop workspace. Inspect resource use, explore process memory information, and export snapshots or process CSV files when you need a record.

Queue your own executable workloads with explicit arguments and review why each job is waiting or allowed to start. VAnta considers available VRAM and your configured reserve before dispatch. View workload output, cancel jobs that VAnta launched, and compare scheduling rules in Policy Lab. A built-in CUDA pulse provides a small optional GPU workload without installing the CUDA Toolkit.

The Store edition runs with your Windows user permissions. Service and startup inventory is read-only, and system cache and standby purges are unavailable. Supported process actions require confirmation and remain subject to Windows access controls. Custom programs run with your permissions and may access files or the network; run only programs you trust.

Live NVIDIA observations and CUDA pulse require compatible NVIDIA hardware and drivers. GPU-dependent features are unavailable without them. VAnta does not install drivers. Observations, workload admission and memory trimming do not guarantee a performance improvement. See the privacy policy for local storage, logs and exports.

**Feature bullets:**

- CPU, RAM, process and optional NVIDIA GPU observations.
- Visible GPU workload queue with VRAM admission explanations.
- Workload output, cancellation and recovery that requires explicit resume.
- Process inspection, memory information and requested snapshot/CSV exports.
- Read-only Windows service and startup inventory.
- Policy Lab for comparing scheduling rules.

## Thai listing

**คำอธิบายสั้น:** ดูทรัพยากรเครื่องและจัดคิวงาน NVIDIA GPU พร้อมเหตุผลการเริ่มงาน

**รายละเอียด:**

VAnta รวมข้อมูล CPU, RAM, โปรเซส และ NVIDIA GPU ที่รองรับไว้ในแอปเดสก์ท็อป Windows ช่วยตรวจการใช้ทรัพยากร ดูข้อมูลหน่วยความจำของโปรเซส และส่งออก snapshot หรือ CSV เมื่อคุณสั่ง

จัดคิวโปรแกรมที่ต้องการรัน พร้อมระบุอาร์กิวเมนต์และตรวจเหตุผลว่างานเริ่มได้หรือยังต้องรอ ระบบพิจารณา VRAM ที่ว่างและส่วนสำรองที่ตั้งไว้ก่อนเริ่มงาน ดูผลลัพธ์ของงาน ยกเลิกงานที่ VAnta เปิด และเปรียบเทียบกติกาจัดคิวผ่าน Policy Lab ได้ มี CUDA pulse เป็นงาน GPU ตัวอย่างแบบเลือกใช้ โดยไม่ต้องติดตั้ง CUDA Toolkit

รุ่น Microsoft Store ทำงานด้วยสิทธิ์ผู้ใช้ Windows รายการ service และ startup ดูได้อย่างเดียว และไม่มีคำสั่งล้าง system cache หรือ standby การจัดการโปรเซสที่รองรับต้องยืนยันและขึ้นอยู่กับสิทธิ์ Windows โปรแกรมที่คุณนำมาจัดคิวทำงานด้วยสิทธิ์ของคุณและอาจเข้าถึงไฟล์หรือเครือข่าย จึงควรใช้เฉพาะโปรแกรมที่เชื่อถือ

ข้อมูล NVIDIA แบบสดและ CUDA pulse ต้องมี GPU และไดรเวอร์ NVIDIA ที่รองรับ ฟีเจอร์เหล่านี้จะใช้ไม่ได้หากไม่มีฮาร์ดแวร์ที่จำเป็น แอปไม่ติดตั้งไดรเวอร์ และไม่รับประกันว่าการตรวจทรัพยากร จัดคิว หรือลด working set จะทำให้เครื่องเร็วขึ้น อินเทอร์เฟซแอปเป็นภาษาอังกฤษ

## Proposed screenshots

Capture the actual Store build with a clean test profile and permission to show any visible data. Review every image for user names, paths, process names, commands and logs before upload.

1. Overview with CPU/RAM and optional NVIDIA observations.
2. Workloads showing a harmless owned test command and its admission explanation.
3. Process/memory inspection with readable labels.
4. Policy Lab showing a deterministic scheduling comparison.
5. Administration showing read-only service/startup behavior.

The current internal development capture is not approved public listing imagery. Check Partner Center's current screenshot dimensions and required fields when preparing the upload.

## Certification notes draft

VAnta is a Rust Win32 desktop application packaged as an x64 MSIX. It declares runFullTrust because its desktop executable reads local process/resource information and launches executable workloads explicitly selected by the user. It runs at medium integrity with the interactive user's permissions and does not request allowElevation. Service control, StartupApproved writes, and system cache/standby mutation are disabled in the Store build, including backend entry points.

No VAnta account or paid service is required. Test basic navigation without NVIDIA hardware; GPU features need compatible NVIDIA hardware/drivers. To inspect observations, launch normally and open Overview, Processes, Memory, Administration and Settings. Policy Lab offers a scheduling comparison. Do not run a real CUDA load or change unrelated processes merely to inspect the app. Any workload/process-action certification test should use a disposable child owned by the tester. Custom workloads are not security-sandboxed.

These notes describe intended behavior and local evidence, not an assertion of Microsoft approval. Attach clean-machine lifecycle results and the final WACK report after they exist.

## Publisher fields still needed

- Actual reserved product name and exact Partner Center package identity/publisher/display name.
- Publisher support contact and public HTTPS privacy-policy URL; finalize the policy contact paragraph before publishing.
- Final category, age rating, markets, price, license terms and supported-language declarations.
- Reviewed screenshots and final public release/package version numbering.
- Installed-package install/update/uninstall evidence, compatibility/accessibility results and WACK report.

Do not paste this document wholesale into public fields: publisher tasks and certification notes belong in their corresponding submission fields.
