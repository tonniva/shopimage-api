# 🎨 Remove Background API

API สำหรับลบพื้นหลังรูปภาพอัตโนมัติด้วย AI + เพิ่มขอบ (optional)

## 📋 Endpoint

```
POST /api/remove-bg
```

## 📥 Request

### Query Parameters (Optional):

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `border` | integer | `0` | ขนาดขอบ (pixels) - 0 = ไม่มีขอบ |
| `border_color` | string | `"white"` | สีขอบ - รองรับ CSS colors |

### Supported Colors:

- `white` - ขาว
- `black` - ดำ
- `red` - แดง
- `blue` - น้ำเงิน
- `green` - เขียว
- `yellow` - เหลือง
- `transparent` - โปร่งใส
- หรือ RGB: `rgb(255, 0, 0)`
- หรือ Hex: `#FF0000`

### Body (multipart/form-data):

```
file: [Image file - JPEG, PNG, WebP]
```

## 📤 Response

### Success (200):

```json
{
  "ok": true,
  "filename": "nobg_image_uuid.png",
  "size_kb": 512,
  "download_url": "http://localhost:8080/dl/output/2024-10-20/nobg_image_uuid.png",
  "quota": {
    "plan": "free",
    "remaining_day": 99,
    "remaining_month": 999
  }
}
```

### Error (400/500):

```json
{
  "ok": false,
  "error": "Error message"
}
```

## 🧪 Examples

### 1. ลบพื้นหลังอย่างเดียว (ไม่มีขอบ):

```bash
curl -X POST "http://localhost:8080/api/remove-bg" \
  -F "file=@product.jpg"
```

### 2. ลบพื้นหลัง + เพิ่มขอบขาว 10px:

```bash
curl -X POST "http://localhost:8080/api/remove-bg?border=10&border_color=white" \
  -F "file=@product.jpg"
```

### 3. ลบพื้นหลัง + เพิ่มขอบดำ 20px:

```bash
curl -X POST "http://localhost:8080/api/remove-bg?border=20&border_color=black" \
  -F "file=@person.jpg"
```

### 4. ลบพื้นหลัง + เพิ่มขอบสีแดง 15px:

```bash
curl -X POST "http://localhost:8080/api/remove-bg?border=15&border_color=red" \
  -F "file=@logo.png"
```

### 5. ลบพื้นหลัง + เพิ่มขอบสี RGB:

```bash
curl -X POST "http://localhost:8080/api/remove-bg?border=10&border_color=rgb(255,100,50)" \
  -F "file=@image.jpg"
```

## 💻 Frontend Examples

### JavaScript (Fetch):

```javascript
async function removeBackground(file, border = 0, borderColor = 'white') {
  const formData = new FormData();
  formData.append('file', file);
  
  const url = `/api/remove-bg?border=${border}&border_color=${borderColor}`;
  
  const response = await fetch(url, {
    method: 'POST',
    body: formData
  });
  
  const result = await response.json();
  
  if (result.ok) {
    // Download หรือแสดงผลลัพธ์
    window.location.href = result.download_url;
  } else {
    alert('Error: ' + result.error);
  }
}

// ใช้งาน
const fileInput = document.querySelector('#fileInput');
removeBackground(fileInput.files[0], 10, 'white');
```

### React Component:

```tsx
import { useState } from 'react';

function RemoveBackgroundTool() {
  const [border, setBorder] = useState(0);
  const [borderColor, setBorderColor] = useState('white');
  const [result, setResult] = useState(null);
  const [loading, setLoading] = useState(false);

  const handleUpload = async (file: File) => {
    setLoading(true);
    
    const formData = new FormData();
    formData.append('file', file);
    
    const url = `/api/remove-bg?border=${border}&border_color=${borderColor}`;
    
    try {
      const response = await fetch(url, {
        method: 'POST',
        body: formData
      });
      
      const data = await response.json();
      setResult(data);
    } catch (error) {
      alert('Error: ' + error.message);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div>
      <h2>Remove Background</h2>
      
      <input 
        type="file" 
        accept="image/*"
        onChange={(e) => handleUpload(e.target.files[0])}
        disabled={loading}
      />
      
      <div>
        <label>
          Border Size (px):
          <input 
            type="number" 
            value={border} 
            onChange={(e) => setBorder(Number(e.target.value))}
            min="0"
            max="100"
          />
        </label>
      </div>
      
      <div>
        <label>
          Border Color:
          <select 
            value={borderColor} 
            onChange={(e) => setBorderColor(e.target.value)}
          >
            <option value="white">White</option>
            <option value="black">Black</option>
            <option value="red">Red</option>
            <option value="blue">Blue</option>
            <option value="green">Green</option>
            <option value="transparent">Transparent</option>
          </select>
        </label>
      </div>
      
      {loading && <p>🔄 Processing...</p>}
      
      {result?.ok && (
        <div>
          <h3>✅ Success!</h3>
          <a href={result.download_url} download>
            📥 Download Image
          </a>
          <img src={result.download_url} alt="Result" />
        </div>
      )}
    </div>
  );
}
```

## ⚙️ Technical Details

### AI Model:
- **Model:** U2-Net (Universal U-Squared Network)
- **Size:** ~176 MB
- **Accuracy:** สูง (F1-score > 0.95)
- **Speed:** 2-10 วินาที/รูป (ขึ้นกับ CPU)

### Output Format:
- **Format:** PNG (รองรับ transparency)
- **Color Mode:** RGBA (ถ้าไม่มีขอบ) หรือ RGB (ถ้ามีขอบสีทึบ)
- **Quality:** Lossless

### Quota:
- นับ 1 quota per request
- เหมือน API อื่นๆ

## 🎯 Use Cases

1. **E-commerce Product Photos:**
   ```bash
   # สินค้าบนพื้นหลังขาว
   ?border=20&border_color=white
   ```

2. **Profile Pictures:**
   ```bash
   # รูปโปรไฟล์ไม่มีพื้นหลัง
   # (ไม่ส่ง parameters)
   ```

3. **Marketing Materials:**
   ```bash
   # รูปสินค้าพร้อมขอบสี brand
   ?border=15&border_color=rgb(255,100,50)
   ```

4. **Thumbnails:**
   ```bash
   # ขอบดำเล็กๆ
   ?border=5&border_color=black
   ```

## 📊 Performance Tips

1. **ขนาดรูป:** ยิ่งเล็กยิ่งเร็ว - แนะนำ resize ก่อน (ถ้าไม่ต้องการความละเอียดสูง)
2. **Format:** JPEG เร็วกว่า PNG
3. **Border:** เพิ่มขอบทำให้ช้าขึ้นนิดหน่อย (< 0.5 วินาที)

## ⚠️ Limitations

- Maximum file size: ตาม `max_upload_mb` (default 8 MB)
- Processing time: 2-15 วินาที (ขึ้นกับขนาดรูป)
- Supported formats: JPEG, PNG, WebP
- Output format: PNG เท่านั้น (เพื่อรองรับ transparency)

