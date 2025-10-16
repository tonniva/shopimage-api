// -------------------- ShopImage: convert_batch.rs --------------------
// รองรับอัปโหลดหลายไฟล์ในคำขอเดียว
// พารามิเตอร์ query ใช้เหมือน /api/convert:
// - platform, target_w, target_h, format, max_upload_mb, max_kb
// รายไฟล์จะถูกตรวจเพดานอัปโหลด และเข้ารหัสตาม format/ขนาดที่ตั้งไว้

use axum::{
    extract::{Multipart, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::Serialize;
use urlencoding::encode;
use uuid::Uuid;

use azure_storage_blobs::prelude::*;
use image::{self, GenericImageView}; // ใช้ image::load_from_memory

use crate::{blob_service_from_env, image_ops, presets, sane_dim, ConvertQuery};

#[derive(Serialize)]
struct BatchItem {
    index: usize,
    ok: bool,
    original_name: Option<String>,
    filename: Option<String>,
    size_kb: Option<u64>,
    download_url: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct BatchResp {
    ok: bool,
    count: usize,
    items: Vec<BatchItem>,
}

// ช่วยแปลง Option<&str> -> Option<u32>, ค่าว่าง "" จะกลายเป็น None
fn parse_u32_opt(s: Option<&str>) -> Option<u32> {
    s.and_then(|v| {
        let t = v.trim();
        if t.is_empty() {
            None
        } else {
            t.parse::<u32>().ok()
        }
    })
}

// base URL สำหรับสร้างลิงก์ /dl (อ่านจาก ENV ได้)
fn api_base() -> String {
    std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".into())
}

pub async fn convert_batch(Query(q): Query<ConvertQuery>, mut mp: Multipart) -> impl IntoResponse {
    // Azure
    let (service, container) = match blob_service_from_env() {
        Ok(v) => v,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let container_client = service.container_client(&container);
    // สร้าง container ถ้ายังไม่มี (ignore 409)
    let _ = container_client
        .create()
        .public_access(PublicAccess::None)
        .into_future()
        .await;

    // -------- พารามิเตอร์ร่วมของ batch นี้ --------
    // เพดานอัปโหลดต่อไฟล์ (MB)
    let max_upload_mb = q.max_upload_mb.unwrap_or(8);
    let max_upload_bytes_per_file: usize = (max_upload_mb as usize) * 1024 * 1024;

    // เพดานเอาต์พุต (KB) -> bytes
    let mut max_bytes = q.max_kb.unwrap_or(2048) * 1024; // default 2MB
                                                         // นามสกุลเอาต์พุต
    let output_format = q.format.as_deref().unwrap_or("webp");

    // width/height — หากไม่ระบุหรือค่าว่าง = None (ใช้ขนาดเดิม)
    // (ConvertQuery ในโปรเจกต์นี้กำหนดให้ target_w/target_h เป็น Option<String>)
    let mut target_w = parse_u32_opt(q.target_w.as_deref()).map(sane_dim);
    let mut target_h = parse_u32_opt(q.target_h.as_deref()).map(sane_dim);

    // preset platform (ถ้ามี)
    if let Some(preset) = q.platform.as_deref().map(presets::preset) {
        max_bytes = q.max_kb.map(|v| v * 1024).unwrap_or(preset.max_bytes);
        if target_w.is_none() {
            target_w = preset.target_w;
        }
        if target_h.is_none() {
            target_h = preset.target_h;
        }
    }

    let mut items: Vec<BatchItem> = Vec::new();
    let mut idx: usize = 0;

    // -------- อ่าน multipart หลายไฟล์ --------
    while let Ok(Some(field)) = mp.next_field().await {
        if field.name() != Some("file") {
            continue;
        }

        let original_name = field.file_name().map(|s| s.to_string());

        // ตรวจขนาดต่อไฟล์
        let data = match field.bytes().await {
            Ok(b) => {
                if b.len() > max_upload_bytes_per_file {
                    items.push(BatchItem {
                        index: idx,
                        ok: false,
                        original_name,
                        filename: None,
                        size_kb: None,
                        download_url: None,
                        error: Some(format!("file too large (max {}MB)", max_upload_mb)),
                    });
                    idx += 1;
                    continue;
                }
                b
            }
            Err(_) => {
                items.push(BatchItem {
                    index: idx,
                    ok: false,
                    original_name,
                    filename: None,
                    size_kb: None,
                    download_url: None,
                    error: Some("Invalid file".into()),
                });
                idx += 1;
                continue;
            }
        };

        // โหลดรูป
        let mut img = match image::load_from_memory(&data) {
            Ok(i) => i,
            Err(_) => {
                items.push(BatchItem {
                    index: idx,
                    ok: false,
                    original_name,
                    filename: None,
                    size_kb: None,
                    download_url: None,
                    error: Some("Unsupported image".into()),
                });
                idx += 1;
                continue;
            }
        };

        // ครอปอัตราส่วน (เฉพาะกรณีกำหนดทั้ง w และ h)
        // if let (Some(w), Some(h)) = (target_w, target_h) {
        //     img = image_ops::ensure_aspect(&img, Some((w, h)));
        // }

        // รีไซซ์ (Behavior A: ถ้าไม่ส่ง w/h เลย -> ใช้รูปเดิม ไม่เรียก resize)
        img = if target_w.is_none() && target_h.is_none() {
            let (ow, oh) = img.dimensions();
            println!(
                "👉 No target size -> using original image size: {}x{}",
                ow, oh
            );
            image_ops::resize_if_needed(&img, Some(ow), Some(oh))
        } else {
            image_ops::resize_if_needed(&img, target_w, target_h)
        };

        // เข้ารหัส
        let (buf, content_type, ext) = match output_format {
            "jpeg" | "jpg" => match image_ops::encode_jpeg_under(&img, max_bytes) {
                Ok(b) => (b, "image/jpeg", "jpg"),
                Err(_) => {
                    items.push(BatchItem {
                        index: idx,
                        ok: false,
                        original_name,
                        filename: None,
                        size_kb: None,
                        download_url: None,
                        error: Some("Image encode failed".into()),
                    });
                    idx += 1;
                    continue;
                }
            },
            _ => match image_ops::encode_webp_under(&img, max_bytes) {
                Ok(b) => (b, "image/webp", "webp"),
                Err(_) => {
                    items.push(BatchItem {
                        index: idx,
                        ok: false,
                        original_name,
                        filename: None,
                        size_kb: None,
                        download_url: None,
                        error: Some("Image encode failed".into()),
                    });
                    idx += 1;
                    continue;
                }
            },
        };

        // อัปโหลดขึ้น Blob
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let filename = format!("{}.{}", Uuid::new_v4(), ext);
        let blob_path = format!("output/{}/{}", today, filename);
        let blob_client = container_client.blob_client(&blob_path);

        match blob_client
            .put_block_blob(buf.clone())
            .content_type(content_type)
            .into_future()
            .await
        {
            Ok(_) => {
                // ส่งเป็น full URL ตาม ENV (เหมือนฝั่ง /api/convert)
                let download_url = format!("{}/dl/{}", api_base(), encode(&blob_path));
                let size_kb = buf.len() as u64 / 1024;
                items.push(BatchItem {
                    index: idx,
                    ok: true,
                    original_name,
                    filename: Some(filename),
                    size_kb: Some(size_kb),
                    download_url: Some(download_url),
                    error: None,
                });
            }
            Err(_) => {
                items.push(BatchItem {
                    index: idx,
                    ok: false,
                    original_name,
                    filename: None,
                    size_kb: None,
                    download_url: None,
                    error: Some("Upload error".into()),
                });
            }
        }

        idx += 1;
    }

    let resp = BatchResp {
        ok: items.iter().any(|i| i.ok),
        count: items.len(),
        items,
    };
    (StatusCode::OK, Json(resp)).into_response()
}
