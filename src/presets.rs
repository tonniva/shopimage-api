use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformPreset {
    pub max_bytes: u64,
    pub target_w: Option<u32>,
    pub target_h: Option<u32>,
    pub aspect: Option<(u32,u32)>, // เช่น 1:1 หรือ 3:4
    pub format: String, // "webp" | "jpeg"
}

pub fn preset(name: &str) -> PlatformPreset {
    match name.to_lowercase().as_str() {
        "shopee" => PlatformPreset {
            max_bytes: 2 * 1024 * 1024, // ~2MB
            target_w: Some(1024),
            target_h: Some(1024),
            aspect: Some((1,1)),
            format: "webp".into(),
        },
        "lazada" => PlatformPreset {
            max_bytes: 3 * 1024 * 1024, // ~3MB
            target_w: Some(1200),
            target_h: Some(1600), // เผื่อ 3:4
            aspect: None,         // ยอมรับทั้ง 1:1 และ 3:4 (จะไม่ crop ถ้าไม่ระบุ)
            format: "webp".into(),
        },
        _ => PlatformPreset {
            max_bytes: 2 * 1024 * 1024,
            target_w: None,
            target_h: None,
            aspect: None,
            format: "webp".into(),
        },
    }
}