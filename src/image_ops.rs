// -------------------- ShopImage: image_ops.rs --------------------
// กลยุทธ์ encode:
// - WebP: เน้นไฟล์เล็ก (ลองคุณภาพต่ำ→สูงแบบพอประมาณ) และลดสเกลหากยังเกิน max_bytes
// - JPEG: ไล่ quality ladder เหมือนกัน และลดสเกลถ้าจำเป็น
// Resize: ไม่ upscale (ไม่ขยายเกินขนาดจริง)
// Ensure_aspect: ครอปกึ่งกลางให้ได้อัตราส่วน w:h ที่ต้องการ

use anyhow::Result;
use image::{DynamicImage, GenericImageView};
use image::imageops::FilterType;
use webp::Encoder;

use image::codecs::jpeg::JpegEncoder;
use std::io::Cursor;

/// ครอปให้ได้อัตราส่วนที่ต้องการ (เช่น target_w:target_h) โดยครอปจากกึ่งกลาง
pub fn ensure_aspect(img: &DynamicImage, want: Option<(u32, u32)>) -> DynamicImage {
    if let Some((aw, ah)) = want {
        let (w, h) = img.dimensions();
        let target = aw as f32 / ah as f32;
        let ratio = w as f32 / h as f32;

        if (ratio - target).abs() > 0.01 {
            if ratio > target {
                // กว้างเกิน → ตัดซ้ายขวา
                let new_w = (h as f32 * target) as u32;
                let x = (w - new_w) / 2;
                return img.crop_imm(x, 0, new_w, h);
            } else {
                // สูงเกิน → ตัดบนล่าง
                let new_h = (w as f32 / target) as u32;
                let y = (h - new_h) / 2;
                return img.crop_imm(0, y, w, new_h);
            }
        }
    }
    img.clone()
}

/// Resize ถ้าจำเป็น "ห้าม upscale" (จะไม่ขยายใหญ่กว่าเดิม)
pub fn resize_if_needed(img: &DynamicImage, target_w: Option<u32>, target_h: Option<u32>) -> DynamicImage {
    let (ow, oh) = img.dimensions();

    match (target_w, target_h) {
        (Some(w), Some(h)) => {
            let w = w.min(ow);
            let h = h.min(oh);
            if w == ow && h == oh { img.clone() }
            else { img.resize_exact(w, h, FilterType::Lanczos3) }
        }
        (Some(w), None) => {
            let w = w.min(ow);
            if w == ow { img.clone() }
            else { img.resize(w, u32::MAX, FilterType::Lanczos3) }
        }
        (None, Some(h)) => {
            let h = h.min(oh);
            if h == oh { img.clone() }
            else { img.resize(u32::MAX, h, FilterType::Lanczos3) }
        }
        _ => img.clone(),
    }
}

/// เข้ารหัสเป็น WebP (lossy) — เน้นไฟล์เล็ก แต่ยังดูดี
/// ขั้นตอน: ลอง quality ต่ำ→สูงแบบพอประมาณในหลายสเกล; ถ้าเกินขนาดให้ลดสเกลลงทีละนิด
pub fn encode_webp_under(img: &DynamicImage, max_bytes: u64) -> Result<Vec<u8>> {
    let quality_steps: [f32; 8] = [35.0, 40.0, 45.0, 50.0, 55.0, 60.0, 70.0, 80.0];
    let mut scale: f32 = 1.0;
    let min_edge: u32 = 320;

    let encode_q = |im: &DynamicImage, q: f32| -> Vec<u8> {
        let rgba = im.to_rgba8();
        let encoder = Encoder::from_rgba(&rgba, im.width(), im.height());
        encoder.encode(q).to_vec()
    };

    loop {
        let candidate = if (scale - 1.0).abs() < f32::EPSILON {
            img.clone()
        } else {
            let (w, h) = img.dimensions();
            let nw = ((w as f32) * scale).round() as u32;
            let nh = ((h as f32) * scale).round() as u32;
            let nw = nw.max(min_edge);
            let nh = nh.max(min_edge);
            img.resize(nw, nh, FilterType::Triangle)
        };

        for &q in &quality_steps {
            let buf = encode_q(&candidate, q);
            if (buf.len() as u64) <= max_bytes {
                return Ok(buf);
            }
        }

        // ยังเกิน -> ลดสเกล
        scale *= 0.85;
        let (w, h) = img.dimensions();
        if (w as f32 * scale).round() as u32 <= min_edge || (h as f32 * scale).round() as u32 <= min_edge {
            // สุดทาง -> คืนแบบคุณภาพต่ำสุดของสเกลสุดท้าย
            let fallback = encode_q(&candidate, quality_steps[0]);
            return Ok(fallback);
        }
    }
}

/// เข้ารหัส JPEG ภายใต้ max_bytes (lossy)
pub fn encode_jpeg_under(img: &DynamicImage, max_bytes: u64) -> Result<Vec<u8>> {
    let mut scale: f32 = 1.0;
    let min_edge: u32 = 320;
    let quality_steps: [u8; 8] = [40, 45, 50, 55, 60, 70, 80, 90];

    let mut candidate = img.clone();

    loop {
        for &q in &quality_steps {
            let mut buf = Vec::new();
            let mut cur = Cursor::new(&mut buf);
            let mut enc = JpegEncoder::new_with_quality(&mut cur, q);
            enc.encode_image(&candidate)?;
            if (buf.len() as u64) <= max_bytes {
                return Ok(buf);
            }
        }

        // ลดสเกล
        scale *= 0.85;
        let (w, h) = candidate.dimensions();
        let nw = ((w as f32) * scale).round() as u32;
        let nh = ((h as f32) * scale).round() as u32;
        if nw <= min_edge || nh <= min_edge {
            let mut buf = Vec::new();
            let mut cur = Cursor::new(&mut buf);
            let mut enc = JpegEncoder::new_with_quality(&mut cur, quality_steps[0]);
            enc.encode_image(&candidate)?;
            return Ok(buf);
        }
        candidate = candidate.resize(nw, nh, FilterType::Triangle);
    }
}