# Mission: ใช้งาน VAnta เพื่อควบคุมงาน GPU

## Why

ผู้ใช้ต้องการใช้ VAnta จัดคิวงาน GPU บน Windows ลดปัญหา VRAM เต็ม งานชนกัน และการตัดสินใจที่อธิบายไม่ได้

## Success looks like

- เปิด VAnta และรัน Demo Scenario ได้
- อธิบายความหมายของสถานะ Queued, Waiting, Running, Completed และ Failed ได้
- เลือกใช้ GPU Pulse, Custom Command และ Policy Lab ได้ถูกต้อง
- อ่าน Decision Feed เพื่อรู้ว่าเหตุใดงานจึงเริ่ม รอ หรือถูกปฏิเสธ

## Constraints

- เรียนรู้จากขั้นตอนที่ทำได้จริงบน Windows
- ใช้ภาษาไทยและตัวอย่างสั้น
- CUDA Pulse ต้องใช้ NVIDIA GPU และไดรเวอร์ที่ใช้งานได้

## Out of scope

- การตั้งค่า NVIDIA driver และ CUDA kernel เชิงลึก
- การพัฒนาโค้ดภายใน VAnta

