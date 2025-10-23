# shopimage-api

## การแก้ไขปัญหา HEIC Format

### ปัญหา
ลูกค้าส่งรูปจาก iPhone แล้วได้ bad request เพราะ iPhone บันทึกรูปในรูปแบบ HEIC (High Efficiency Image Container) ซึ่งไม่รองรับโดยตรงในระบบ

### การแก้ไข
1. **เพิ่มการตรวจสอบ HEIC format** - ระบบจะตรวจสอบว่าไฟล์ที่ส่งมาเป็น HEIC หรือไม่
2. **แสดงข้อความแจ้งเตือนที่ชัดเจน** - เมื่อพบไฟล์ HEIC จะแสดงข้อความแนะนำให้แปลงเป็น JPEG หรือ PNG ก่อน
3. **รองรับ format อื่นๆ** - ยังคงรองรับ JPEG, PNG, WebP ตามปกติ

### วิธีแก้ไขสำหรับลูกค้า
ลูกค้าสามารถแก้ไขได้โดย:
1. **เปลี่ยนการตั้งค่า iPhone**: ไปที่ การตั้งค่า > กล้อง > รูปแบบ > เลือก "เข้ากันได้มากที่สุด"
2. **แปลงไฟล์ HEIC**: ใช้แอปแปลงไฟล์ใน iPhone หรือเครื่องมือออนไลน์

### รองรับ Format
- ✅ JPEG
- ✅ PNG  
- ✅ WebP
- ❌ HEIC (แสดงข้อความแนะนำให้แปลง)
- 🔄 PDF (API endpoint พร้อม แต่ยังไม่รองรับการแปลง)

## การเพิ่ม PDF Support

### API Endpoint ใหม่
- **POST /api/convert-pdf** - แปลง PDF เป็นรูปภาพ JPG

### พารามิเตอร์
- `file` - ไฟล์ PDF ที่ต้องการแปลง
- `page` - หน้าที่ต้องการแปลง (เริ่มจาก 0, default = 0)
- `format` - รูปแบบเอาต์พุต ("jpeg" default, "webp")
- `target_w`, `target_h` - ขนาดที่ต้องการ
- `max_kb` - ขนาดไฟล์สูงสุด (KB)
- `max_upload_mb` - ขนาดไฟล์อัปโหลดสูงสุด (MB)

### สถานะปัจจุบัน
- ✅ API endpoint พร้อมใช้งาน
- ✅ การตรวจสอบ PDF format
- ✅ การจัดการ quota และ rate limiting
- ✅ การแปลง PDF เป็นรูปภาพ (ใช้ pdftoppm)


### วิธีใช้งาน

#### แปลงหน้าเดียว:
```bash
curl -X POST "http://localhost:8080/api/convert-pdf" \
  -F "file=@document.pdf" \
  -F "page=0" \
  -F "format=jpeg" \
  -F "target_w=800" \
  -F "target_h=600"
```

#### แปลงทุกหน้า:
```bash
curl -X POST "http://localhost:8080/api/convert-pdf-all" \
  -F "file=@document.pdf" \
  -F "format=jpeg" \
  -F "target_w=800" \
  -F "target_h=600"
```

### การแก้ไขปัญหา "PDF conversion failed"

#### ถ้าได้ข้อผิดพลาด "Failed to run pdftoppm":
1. **ติดตั้ง poppler-utils**:
   ```bash
   # macOS
   brew install poppler
   
   # Ubuntu/Debian
   sudo apt-get install poppler-utils
   
   # CentOS/RHEL
   sudo yum install poppler-utils
   ```

2. **ตรวจสอบการติดตั้ง**:
   ```bash
   pdftoppm -h
   ```

#### ถ้าได้ข้อผิดพลาดอื่น:
- ตรวจสอบว่าไฟล์ PDF ไม่เสียหาย
- ลองใช้หน้าแรก (page=0)
- ตรวจสอบ logs ของเซิร์ฟเวอร์

### หมายเหตุ
- API endpoint พร้อมใช้งานและสามารถแปลง PDF เป็นรูปภาพได้
- ใช้ pdftoppm จาก poppler-utils สำหรับการแปลง
- รองรับการเลือกหน้าที่ต้องการแปลง (page parameter)
- รองรับการแปลงทุกหน้าใน PDF (ใช้ `/api/convert-pdf-all`)
- ระบบจะลบไฟล์ชั่วคราวหลังจากแปลงเสร็จ
