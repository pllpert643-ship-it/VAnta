# Task Manager ใน VAnta

## เปิดใช้งาน

เปิด shortcut **VAnta** บน Desktop ตามเดิม รุ่นนี้เปิดแท็บ **Task Manager** เป็นหน้าแรก ส่วน dashboard เดิมอยู่ที่ **GPU Workloads**

ไฟล์โปรแกรม: `dist/VAnta/VAnta.exe`

## สิ่งที่ใช้งานได้

- **Processes / Details:** รายการโปรเซสจริง ค้นหา เรียงตัวเลข เลือกคอลัมน์ และคัดลอกตาราง มีชื่อ PID สถานะ CPU RAM I/O ผู้ใช้ threads handles CPU time private bytes parent PID session priority executable command line และตัวนับ GPU ของ Windows
- **กลุ่มโปรเซส:** แยกแอปที่มีหน้าต่าง โปรเซสที่ไฟล์อยู่ใต้ Windows และ background เป็นคอลัมน์ Group เป็นการจัดประเภทของ VAnta ไม่ใช่ application grouping แบบ Windows Task Manager
- **คำสั่งโปรเซส:** End task / tree, priority, affinity, เปิด/ปิด Efficiency mode, switch to window, wait chain, process memory dump, ตำแหน่งไฟล์ Properties ค้นหาออนไลน์ ไป Details/Services และ restart Explorer
- **Performance:** กราฟ CPU/RAM, CPU ราย logical processor, uptime, RAM, committed/commit limit/cache/kernel pools, disk I/O, network adapters, NVIDIA GPU อุณหภูมิ/พลังงาน/หน่วยความจำ และ GPU engine ของ Windows
- **Hardware:** รายการ CPU, RAM modules/สล็อตที่เฟิร์มแวร์รายงาน, GPU/driver และดิสก์จาก CIM ตัวนับ GPU engine ใช้ Windows LUID เพื่อไม่จับคู่ชื่อการ์ดผิด
- **App history:** ประวัติ CPU และ I/O ที่ VAnta สังเกตเอง บันทึกลง SQLite และล้างประวัติได้ เริ่มนับจากผลต่างสองตัวอย่าง ไม่เอาการใช้งานตลอดอายุโปรเซสมานับเป็นประวัติใหม่
- **Startup apps:** อ่าน Run keys 32/64-bit และ Startup folders ปิดรายการโดยสำรองไว้ใน `%LOCALAPPDATA%/VAnta/disabled_startup` และเปิดคืนจากสำเนาเดิมได้ ไม่เขียนทับรายการที่สร้างใหม่ตรงชื่อ/ตำแหน่งเดิม
- **Users:** session ผู้ใช้จริงและทรัพยากรที่อ่านได้ ดูโปรเซส Disconnect / Sign out โดยถามยืนยันก่อน
- **Services:** รายการ service จริง Start / Stop / Restart ไปยังโปรเซสและเปิด Services console
- **Settings:** อัตราอัปเดต 1/2/4 วินาทีหรือ Pause, default page, always on top, การถามยืนยัน Efficiency mode และจำคอลัมน์ที่ซ่อน
- **Run new task:** เปิดโปรแกรมพร้อม arguments และขอสิทธิ์ administrator ผ่าน UAC ได้

## วิธีลอง

1. เปิด VAnta แล้วรอสองรอบการอัปเดตให้ CPU และอัตรา I/O มี baseline
2. เปิด Notepad เอง ค้นหา `notepad` ใน VAnta และดู PID
3. เลือกโปรเซส คลิก **More actions...** หรือคลิกขวา เพื่อลองเปิดตำแหน่งไฟล์ Properties priority/affinity หรือ End task บนแอปทดสอบของคุณ
4. เปิด **Performance** ดู CPU/Memory/GPU engines/Hardware
5. เปิด Users และ Services เพื่อตรวจรายการ คำสั่งเปลี่ยนแปลงจะถามก่อนดำเนินการ
6. กลับ **GPU Workloads** เพื่อใช้ dashboard และคิว GPU เดิม

## ขอบเขตที่ยังไม่ครบเทียบกับ Windows Task Manager

รุ่นนี้ยังไม่ใช่ full parity ทุกฟังก์ชันของ Task Manager:

- App history ไม่ได้ดึง SRUM/ประวัติ metered network ของ Windows และไม่มี network bytes รายโปรเซส
- Startup ยังไม่มี startup impact / Last BIOS time / packaged startup tasks ในตัว รายการที่ Windows ปิดเองต้องใช้ปุ่ม Windows Startup apps
- ไม่มี live kernel dump ในตัว, NPU, UAC virtualization, รายละเอียดบางคอลัมน์ของ Windows, power usage/trend, light/system theme หรือ tray/minimize settings
- ยังไม่มี application process tree grouping แบบ Windows, compact graph view, GPU engine graph ย้อนหลังแยกทุก engine หรือ disk active/response time
- GPU temperature/power ใช้ NVIDIA NVML; WDDM engine/memory ใช้ตัวนับ Windows และอาจไม่มีข้อมูลบนบางไดรเวอร์ ข้อมูลที่ไม่รองรับแสดง Unavailable/รายการว่าง ไม่สมมติเป็นข้อมูลที่วัดแล้ว
- hardware metadata พึ่งข้อมูลที่เฟิร์มแวร์ให้ ไม่ได้บอกว่าชิ้นส่วนเสียหรือเป็นคะแนน benchmark
- ข้อมูลโปรเซสที่ Windows จำกัดสิทธิ์จะแสดง restricted; คำสั่งอาจถูกปฏิเสธแม้รายการโปรเซสยังมองเห็น
- VAnta ป้องกันการสั่งเปลี่ยนตัวเองและ core Windows processes; ตรวจ PID พร้อม creation time ก่อนเปลี่ยนโปรเซสเพื่อป้องกัน PID reuse
- เปิดปุ่ม **Windows Task Manager** เพื่อใช้ฟังก์ชันเฉพาะ Windows ที่ยังไม่ได้ทำใน VAnta

## การตรวจสอบก่อนส่ง

- ชุดทดสอบ pytest ครอบคลุม sampler, PID reuse, การป้องกัน self-termination, history delta/reset, startup backup/restore และการไม่เขียนทับไฟล์ใหม่
- Windows integration ใช้ child process ที่สร้างเพื่อทดสอบเท่านั้น: priority, affinity, EcoQoS, wait chain และ memory dump พร้อมตรวจ MDMP signature
- ทดสอบเปิดข้อมูลครบแปดหน้า และเปิด MainWindow จริงพร้อม dashboard เดิม ตรวจ numeric sorting และปิด worker โดยไม่ทิ้ง thread ค้าง
- ไม่ได้ทดลอง Sign out/Disconnect ของผู้ใช้จริง หรือ Start/Stop services/ปิด startup ของเครื่องผู้ใช้

- ??????????????? 24 ?????? ??????????? `.exe` ???????????: ??????????????? ?????????????????????? SQLite ????????????????????
- ????? `dist/VAnta-windows-x64.zip` ?????????????????????? ?????????????? executable ?? ZIP ???????????????????
