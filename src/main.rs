// -------------------- ShopImage: main.rs --------------------
// พารามิเตอร์ API:
// - target_w, target_h : ขนาดที่ต้องการ (ไม่ส่ง/ค่าว่าง = ใช้ขนาดเดิม)
// - format             : "webp" (default) | "jpeg"
// - max_upload_mb      : เพดานไฟล์อัปโหลดต่อไฟล์ (default 8MB)
// - max_kb             : เพดานขนาดเอาต์พุต (KB) สำหรับ encoder (default 2048KB)
// เมื่อส่ง target_w + target_h พร้อมกัน -> ครอปกึ่งกลางให้ได้อัตราส่วน แล้วค่อยรีไซส์ (ไม่ upscale)
// อัปโหลดขึ้น Azure Blob แบบ private และคืนลิงก์ผ่าน /dl/... (proxy) เพื่อซ่อน Blob URL จริง

mod convert_batch;
mod image_ops;
mod presets;
mod quota;
mod rate_limit;

use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, Multipart, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use image::GenericImageView;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tower_http::{
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    trace::TraceLayer,
};
use urlencoding::encode;
use uuid::Uuid;

// Azure SDK
use azure_storage::prelude::StorageCredentials;
use azure_storage_blobs::prelude::*;

// PDF manipulation
use lopdf::Document;

// Redis caching

// ---------- serde helper: ค่าว่าง ("") -> None และ parse เป็นตัวเลข ----------
use serde::de::Deserializer;
use std::{fmt::Display, str::FromStr};

fn opt_num_from_str<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: Display,
{
    let opt = Option::<String>::deserialize(de)?;
    match opt {
        None => Ok(None),
        Some(s) => {
            let s = s.trim();
            if s.is_empty() {
                Ok(None)
            } else {
                s.parse::<T>().map(Some).map_err(serde::de::Error::custom)
            }
        }
    }
}

// -------------------- Query / Response --------------------

#[derive(Deserialize)]
pub struct ConvertQuery {
    platform: Option<String>,

    #[serde(default, deserialize_with = "opt_num_from_str")]
    max_kb: Option<u64>, // KB

    format: Option<String>, // "webp" (default) | "jpeg"

    // รองรับทั้ง "ไม่ส่ง" และ "ส่งเป็นค่าว่าง"
    target_w: Option<String>,
    target_h: Option<String>,

    #[serde(default, deserialize_with = "opt_num_from_str")]
    max_upload_mb: Option<u64>,

    // สำหรับ remove-bg: ขนาดและสีของขอบ
    #[serde(default, deserialize_with = "opt_num_from_str")]
    border: Option<u64>, // ขนาดขอบ (pixels)

    border_color: Option<String>, // สีขอบ (white, black, red, etc.)
}

#[derive(Serialize, Debug)]
struct QuotaPayload {
    pub plan: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_day: Option<u64>, // None = unlimited (pro/business)
    pub remaining_month: u64,
}

#[derive(Serialize, Debug)]
struct ConvertResp {
    ok: bool,
    filename: String,
    size_kb: u64,
    download_url: String,
    download_url_array: Option<Vec<String>>, // สำหรับ PDF ที่มีหลายหน้า
    quota: QuotaPayload,                     // ส่งเสมอ
}

fn parse_u32_opt(s: Option<&str>) -> Option<u32> {
    s.and_then(|v| {
        let t = v.trim();
        if t.is_empty() {
            None
        } else {
            v.parse::<u32>().ok()
        }
    })
}

// -------------------- App State --------------------

#[derive(Clone)]
struct AppState {
    quota: Arc<quota::Quota>,          // โควตาต่อ user/ip
}

// -------------------- App / Main --------------------

fn app() -> Router {
    // IMPORTANT: expose headers เพื่อให้ frontend อ่าน getResponseHeader ได้
    let cors = CorsLayer::new()
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_origin(Any)
        .expose_headers([
            axum::http::header::HeaderName::from_static("x-quota-plan"),
            axum::http::header::HeaderName::from_static("x-quota-remaining-day"),
            axum::http::header::HeaderName::from_static("x-quota-remaining-month"),
            axum::http::header::HeaderName::from_static("x-ratelimit-limit"),
            axum::http::header::HeaderName::from_static("x-ratelimit-remaining"),
            axum::http::header::HeaderName::from_static("retry-after"),
        ]);

    // อ่านค่าจาก ENV ได้ (ถ้าไม่ตั้ง จะใช้ดีฟอลต์)
    let limit_per_min: u32 = std::env::var("RATE_LIMIT_PER_MIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);

    let lock_secs: u64 = std::env::var("RATE_LIMIT_LOCK_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0); // 0 = ปิด lock

    let rl_state =
        rate_limit::RateLimitState::new_with_lock(limit_per_min, lock_secs > 0, lock_secs);

    // ✅ สร้าง AppState (Quota memory + Redis cache)
    let state = AppState {
        quota: Arc::new(quota::Quota::new()),
    };

    // เส้นที่ต้อง rate-limit (ต่อ IP)
    let protected = Router::new()
        .route("/api/convert", post(convert))
        .route("/api/convert-batch", post(convert_batch::convert_batch))
        .route("/api/convert-pdf", post(convert_pdf))
        .route("/api/convert-pdf-all", post(convert_pdf_all))
        .route("/api/merge-pdf", post(merge_pdf))
        .route("/api/remove-bg", post(remove_background))
        .route_layer(middleware::from_fn_with_state(
            rl_state.clone(),
            rate_limit::rate_limit,
        ));

    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/dl/*path", get(download_from_blob))
        .merge(protected)
        .with_state(state) // ✅ แนบ state (Quota) เข้ากับทุกเส้นทาง
        .layer(RequestBodyLimitLayer::new(64 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080);
    println!("BOOT: PORT={}", port);
    println!(
        "BOOT: AZURE_BLOB_CONTAINER={}",
        std::env::var("AZURE_BLOB_CONTAINER").unwrap_or_default()
    );
    println!(
        "BOOT: API_BASE_URL={}",
        std::env::var("API_BASE_URL").unwrap_or_default()
    );

    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).await.unwrap();
    println!("✅ ShopImage API started at http://{addr}");

    axum::serve(
        listener,
        app().into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}
// -------------------- Helpers --------------------

pub fn sane_dim(v: u32) -> u32 {
    v.clamp(1, 4096)
}

fn api_base() -> String {
    std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".into())
}

fn guess_content_type_by_ext(path: &str) -> &'static str {
    if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".pdf") {
        "application/pdf"
    } else {
        "image/webp"
    }
}

fn parse_conn_string(conn: &str) -> Option<(String, String)> {
    let mut acc = None::<String>;
    let mut key = None::<String>;
    for part in conn.split(';') {
        let mut it = part.splitn(2, '=');
        let k = it.next()?.trim();
        let v = it.next().unwrap_or("").trim().to_string();
        match k {
            "AccountName" => acc = Some(v),
            "AccountKey" => key = Some(v),
            _ => {}
        }
    }
    match (acc, key) {
        (Some(a), Some(k)) => Some((a, k)),
        _ => None,
    }
}

/// ตรวจสอบและแปลง HEIC/PDF format เป็น DynamicImage
fn load_image_with_heic_support(data: &[u8]) -> Result<image::DynamicImage, String> {
    // ลองโหลดด้วย image crate ก่อน (รองรับ JPEG, PNG, WebP)
    if let Ok(img) = image::load_from_memory(data) {
        return Ok(img);
    }

    // ถ้าไม่ได้ ลองตรวจสอบว่าเป็น HEIC format หรือไม่
    if is_heic_format(data) {
        return Err(
            "HEIC format detected but not supported. Please convert to JPEG or PNG format first."
                .to_string(),
        );
    }

    // ถ้าไม่ได้ ลองตรวจสอบว่าเป็น PDF format หรือไม่
    if is_pdf_format(data) {
        return Err(
            "PDF format detected. Please use /api/convert-pdf endpoint for PDF conversion."
                .to_string(),
        );
    }

    Err("Unsupported image format. Supported formats: JPEG, PNG, WebP".to_string())
}

/// ตรวจสอบว่าไฟล์เป็น HEIC format หรือไม่
fn is_heic_format(data: &[u8]) -> bool {
    // HEIC files start with specific bytes
    if data.len() < 12 {
        return false;
    }

    // Check for HEIC file signature
    // HEIC files typically start with "ftyp" box
    for i in 0..data.len().saturating_sub(8) {
        if &data[i..i + 4] == b"ftyp" {
            // Check for HEIC brand
            if data.len() > i + 8 {
                let brand = &data[i + 4..i + 8];
                if brand == b"heic" || brand == b"heix" || brand == b"hevc" || brand == b"hevx" {
                    return true;
                }
            }
        }
    }

    false
}

/// ตรวจสอบว่าไฟล์เป็น PDF format หรือไม่
fn is_pdf_format(data: &[u8]) -> bool {
    // PDF files start with "%PDF-"
    if data.len() < 5 {
        return false;
    }

    &data[0..5] == b"%PDF-"
}

pub fn blob_service_from_env() -> Result<(BlobServiceClient, String), String> {
    let conn = std::env::var("AZURE_STORAGE_CONNECTION_STRING")
        .map_err(|_| "Missing AZURE_STORAGE_CONNECTION_STRING".to_string())?;
    let container = std::env::var("AZURE_BLOB_CONTAINER").unwrap_or_else(|_| "shopimage".into());

    let (account, key) = parse_conn_string(&conn).ok_or("Invalid connection string".to_string())?;
    let creds = StorageCredentials::access_key(account.clone(), key);
    let service = BlobServiceClient::new(account, creds);
    Ok((service, container))
}

// ---------- auth/quota helpers ----------

// เลือกตัวตนที่มีผล: ถ้ามี x-user-id ใช้นั้น, ถ้าไม่มีใช้ IP เป็น guest
fn effective_user(headers: &HeaderMap, remote: Option<SocketAddr>) -> (String, String) {
    if let Some(uid) = headers.get("x-user-id").and_then(|v| v.to_str().ok()) {
        let plan = headers
            .get("x-plan")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("free");
        return (uid.to_string(), plan.to_string());
    }

    // ใช้ IP เป็นตัวตน (guest)
    let ip_from_xff = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string());

    let ip = ip_from_xff
        .or_else(|| remote.map(|sa| sa.ip().to_string()))
        .unwrap_or_else(|| "unknown".to_string());

    (format!("ip:{ip}"), "free".to_string())
}

fn add_quota_headers(resp: &mut axum::response::Response, qr: &quota::QuotaResult) {
    let h = resp.headers_mut();
    h.insert("x-quota-plan", HeaderValue::from_str(&qr.plan).unwrap());
    if let Some(rd) = qr.remaining_day {
        h.insert(
            "x-quota-remaining-day",
            HeaderValue::from_str(&rd.to_string()).unwrap(),
        );
    } else {
        h.insert(
            "x-quota-remaining-day",
            HeaderValue::from_static("unlimited"),
        );
    }
    h.insert(
        "x-quota-remaining-month",
        HeaderValue::from_str(&qr.remaining_month.to_string()).unwrap(),
    );
}

// -------------------- Handlers --------------------

async fn convert(
    State(st): State<AppState>,
    headers: HeaderMap,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Query(q): Query<ConvertQuery>,
    mut mp: Multipart,
) -> impl IntoResponse {
    // ✅ ใช้ตัวตนที่มีผล (user id หรือ ip)
    let (user_id, plan) = effective_user(&headers, Some(remote));

    // เตรียม Azure
    let (service, container) = match blob_service_from_env() {
        Ok(v) => v,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let container_client = service.container_client(&container);
    let _ = container_client
        .create()
        .public_access(PublicAccess::None)
        .into_future()
        .await;

    // 1) รับ/คุมพารามิเตอร์
    let max_upload_mb = q.max_upload_mb.unwrap_or(8);
    let max_upload_bytes_per_file: usize = (max_upload_mb as usize) * 1024 * 1024;

    let max_bytes = q.max_kb.unwrap_or(2048) * 1024; // default 2MB
    let output_format = q.format.as_deref().unwrap_or("webp");

    let target_w = parse_u32_opt(q.target_w.as_deref()).map(sane_dim);
    let target_h = parse_u32_opt(q.target_h.as_deref()).map(sane_dim);

    // สำหรับ PDF conversion ทุกหน้า

    // 2) อ่าน multipart: field ชื่อ "file"
    while let Ok(Some(field)) = mp.next_field().await {
        if field.name() == Some("file") {
            // ✅ ขอใช้โควตา 1 ภาพก่อนเริ่มทำ
            let qr = st.quota.try_consume(&user_id, 1, &plan);
            if !qr.allowed {
                let body = serde_json::json!({
                    "ok": false,
                    "error": qr.message.as_deref().unwrap_or("quota exceeded"),
                    "quota": {
                        "plan": qr.plan,
                        "remaining_day": qr.remaining_day,
                        "remaining_month": qr.remaining_month
                    }
                });
                let mut resp = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
                add_quota_headers(&mut resp, &qr);
                return resp;
            }

            // เช็กขนาดต่อไฟล์
            let data = match field.bytes().await {
                Ok(b) => {
                    if b.len() > max_upload_bytes_per_file {
                        return (
                            StatusCode::PAYLOAD_TOO_LARGE,
                            format!("file too large (max {}MB)", max_upload_mb),
                        )
                            .into_response();
                    }
                    b
                }
                Err(_) => return (StatusCode::BAD_REQUEST, "Invalid file").into_response(),
            };

            // โหลดรูป (รองรับ HEIC จาก iPhone)
            let img = match load_image_with_heic_support(&data) {
                Ok(i) => i,
                Err(e) => {
                    eprintln!("Image load error: {}", e);
                    return (
                        StatusCode::UNSUPPORTED_MEDIA_TYPE,
                        "Unsupported image format",
                    )
                        .into_response();
                }
            };

            let mut img = img;

            // 3) ครอปสัดส่วน (ถ้ากำหนดทั้ง w+h)
            if let (Some(w), Some(h)) = (target_w, target_h) {
                img = image_ops::ensure_aspect(&img, Some((w, h)));
            }

            // 4) รีไซส์ — ถ้าไม่ระบุเลย ใช้ขนาดเดิม
            img = if target_w.is_none() && target_h.is_none() {
                let (ow, oh) = img.dimensions();
                image_ops::resize_if_needed(&img, Some(ow), Some(oh))
            } else {
                image_ops::resize_if_needed(&img, target_w, target_h)
            };

            // 5) เข้ารหัสตาม format + คุมขนาด (≤ max_bytes)
            let (buf, content_type, ext) = match output_format {
                "jpeg" | "jpg" => match image_ops::encode_jpeg_under(&img, max_bytes) {
                    Ok(b) => (b, "image/jpeg", "jpg"),
                    Err(_) => {
                        return (StatusCode::INTERNAL_SERVER_ERROR, "Image encode failed")
                            .into_response()
                    }
                },
                _ => match image_ops::encode_webp_under(&img, max_bytes) {
                    Ok(b) => (b, "image/webp", "webp"),
                    Err(_) => {
                        return (StatusCode::INTERNAL_SERVER_ERROR, "Image encode failed")
                            .into_response()
                    }
                },
            };

            // 6) อัปโหลดขึ้น Blob
            let today = Utc::now().format("%Y-%m-%d").to_string();
            let filename = format!("{}.{}", Uuid::new_v4(), ext);
            let blob_path = format!("output/{}/{}", today, filename);

            let blob_client = container_client.blob_client(&blob_path);
            if let Err(e) = blob_client
                .put_block_blob(buf.clone())
                .content_type(content_type)
                .into_future()
                .await
            {
                eprintln!("upload error: {e:?}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Upload error").into_response();
            }

            // 7) ส่งผลลัพธ์ (proxy URL ผ่าน /dl) + แนบ quota headers
            let download_url = format!("{}/dl/{}", api_base(), encode(&blob_path));
            let size_kb = buf.len() as u64 / 1024;

            let mut resp = Json(ConvertResp {
                ok: true,
                filename,
                size_kb,
                download_url,
                download_url_array: None, // รูปภาพเดี่ยวไม่มี array
                quota: QuotaPayload {
                    plan: qr.plan.clone(),
                    remaining_day: qr.remaining_day,
                    remaining_month: qr.remaining_month,
                },
            })
            .into_response();

            add_quota_headers(&mut resp, &qr);
            return resp;
        }
    }

    (StatusCode::BAD_REQUEST, "No file received").into_response()
}

// -------------------- Remove Background Handler --------------------

/// Remove background from image using Python rembg
async fn remove_background(
    State(st): State<AppState>,
    headers: HeaderMap,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Query(q): Query<ConvertQuery>,
    mut mp: Multipart,
) -> impl IntoResponse {
    // ✅ ใช้ตัวตนที่มีผล (user id หรือ ip)
    let (user_id, plan) = effective_user(&headers, Some(remote));

    // ตรวจ quota
    let qr = st.quota.try_consume(&user_id, 1, &plan);
    if !qr.allowed {
        let body = serde_json::json!({
            "ok": false,
            "error": qr.message.unwrap_or_else(|| "Quota exceeded".to_string())
        });
        return (StatusCode::FORBIDDEN, Json(body)).into_response();
    }

    // ได้ Blob Service Client
    let (blob_service_client, container_name) = match blob_service_from_env() {
        Ok(v) => v,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };

    let container_client = blob_service_client.container_client(&container_name);

    // อ่านไฟล์รูปภาพ
    while let Some(field) = mp.next_field().await.unwrap_or(None) {
        let filename = field.file_name().unwrap_or("unknown").to_string();

        let data = match field.bytes().await {
            Ok(d) => d.to_vec(),
            Err(_) => continue,
        };

        if data.is_empty() {
            continue;
        }

        eprintln!("🖼️ Removing background from: {}", filename);

        // ดึงค่า border parameters
        let border_size = q.border.unwrap_or(0);
        let border_color = q.border_color.as_deref().unwrap_or("white");

        eprintln!("  🎨 Border: {} px, Color: {}", border_size, border_color);

        // ลบพื้นหลังด้วย Python script
        let output_data = match remove_bg_with_python(&data, border_size, border_color) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("❌ Failed to remove background: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to remove background: {}", e),
                )
                    .into_response();
            }
        };

        // อัปโหลดผลลัพธ์
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let output_filename = format!(
            "nobg_{}_{}.png",
            filename.trim_end_matches(&['.', 'j', 'p', 'g', 'J', 'P', 'G', 'n', 'e', 'w'][..]),
            Uuid::new_v4()
        );
        let blob_path = format!("output/{}/{}", today, output_filename);

        let blob_client = container_client.blob_client(&blob_path);
        if let Err(e) = blob_client
            .put_block_blob(output_data.clone())
            .content_type("image/png")
            .into_future()
            .await
        {
            eprintln!("upload error: {e:?}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Upload error").into_response();
        }

        let download_url = format!("{}/dl/{}", api_base(), encode(&blob_path));
        let size_kb = output_data.len() as u64 / 1024;

        eprintln!("✅ Background removed successfully!");

        // ส่งผลลัพธ์
        let mut resp = Json(ConvertResp {
            ok: true,
            filename: output_filename,
            size_kb,
            download_url,
            download_url_array: None,
            quota: QuotaPayload {
                plan: qr.plan.clone(),
                remaining_day: qr.remaining_day,
                remaining_month: qr.remaining_month,
            },
        })
        .into_response();
        add_quota_headers(&mut resp, &qr);
        return resp;
    }

    (StatusCode::BAD_REQUEST, "No file received").into_response()
}

/// Remove background using Python rembg library
fn remove_bg_with_python(
    image_data: &[u8],
    border_size: u64,
    border_color: &str,
) -> Result<Vec<u8>, String> {
    use std::fs;
    use std::process::Command;

    let temp_dir = std::env::temp_dir();
    let input_path = temp_dir.join(format!("input_{}.jpg", Uuid::new_v4()));
    let output_path = temp_dir.join(format!("output_{}.png", Uuid::new_v4()));

    // บันทึก input file
    fs::write(&input_path, image_data).map_err(|e| format!("Failed to write temp file: {}", e))?;

    eprintln!("🔄 Running Python rembg script...");

    // สร้าง command พร้อม arguments
    let mut cmd = Command::new("python3");
    cmd.arg("remove_bg.py").arg(&input_path).arg(&output_path);

    // เพิ่ม border parameters ถ้ามี
    if border_size > 0 {
        cmd.arg("--border").arg(border_size.to_string());
        cmd.arg("--border-color").arg(border_color);
        eprintln!("  🎨 Adding {}px {} border", border_size, border_color);
    }

    // เรียก Python script
    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run Python script: {}. Please install python3 and rembg (pip3 install rembg)", e))?;

    if !output.status.success() {
        // ลบ temp files
        let _ = fs::remove_file(&input_path);
        let _ = fs::remove_file(&output_path);
        return Err(format!(
            "Python script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // อ่าน output file
    let result_data =
        fs::read(&output_path).map_err(|e| format!("Failed to read output file: {}", e))?;

    // ลบ temp files
    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);

    Ok(result_data)
}

// -------------------- PDF Merge Handler --------------------

/// Merge multiple PDFs into one
async fn merge_pdf(
    State(st): State<AppState>,
    headers: HeaderMap,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Query(_q): Query<ConvertQuery>,
    mut mp: Multipart,
) -> impl IntoResponse {
    // ✅ ใช้ตัวตนที่มีผล (user id หรือ ip)
    let (user_id, plan) = effective_user(&headers, Some(remote));

    // ตรวจ quota
    let qr = st.quota.try_consume(&user_id, 1, &plan);
    if !qr.allowed {
        let body = serde_json::json!({
            "ok": false,
            "error": qr.message.unwrap_or_else(|| "Quota exceeded".to_string())
        });
        return (StatusCode::FORBIDDEN, Json(body)).into_response();
    }

    // ได้ Blob Service Client
    let (blob_service_client, container_name) = match blob_service_from_env() {
        Ok(v) => v,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };

    let container_client = blob_service_client.container_client(&container_name);

    let mut pdf_documents: Vec<(String, Vec<u8>)> = Vec::new();

    // อ่านไฟล์ PDF ทั้งหมดจาก multipart
    while let Some(field) = mp.next_field().await.unwrap_or(None) {
        let field_name = field.name().unwrap_or("").to_string();

        eprintln!("🔍 DEBUG: Field name: '{}'", field_name);

        // ข้าม field ที่เป็น metadata หรือ parameters
        if field_name.is_empty() {
            continue;
        }

        let filename = field.file_name().unwrap_or("").to_string();

        eprintln!("📄 DEBUG: Filename: '{}'", filename);

        // ถ้าไม่มี filename แสดงว่าไม่ใช่ file field (เป็น text field)
        if filename.is_empty() {
            eprintln!("⏭️ DEBUG: Skipping non-file field");
            continue;
        }

        let data = match field.bytes().await {
            Ok(d) => d.to_vec(),
            Err(e) => {
                eprintln!("❌ DEBUG: Failed to read bytes: {:?}", e);
                continue;
            }
        };

        eprintln!("📦 DEBUG: Data size: {} bytes", data.len());

        // ข้าม field ที่ว่างเปล่า
        if data.is_empty() {
            eprintln!("⏭️ DEBUG: Skipping empty file");
            continue;
        }

        eprintln!(
            "🔍 DEBUG: First 10 bytes: {:?}",
            &data.get(0..10.min(data.len()))
        );

        // ตรวจสอบว่าเป็น PDF
        if !is_pdf_format(&data) {
            eprintln!("❌ DEBUG: Not a valid PDF format");
            return (
                StatusCode::BAD_REQUEST,
                format!("File '{}' is not a valid PDF", filename),
            )
                .into_response();
        }

        eprintln!("✅ DEBUG: Valid PDF added: '{}'", filename);
        pdf_documents.push((filename, data));
    }

    // ต้องมีอย่างน้อย 2 ไฟล์
    if pdf_documents.len() < 2 {
        return (
            StatusCode::BAD_REQUEST,
            "At least 2 PDF files are required for merging",
        )
            .into_response();
    }

    // Merge PDFs using Python script (รองรับภาษาไทย 100%)
    let merged_pdf = match merge_pdfs_with_python(&pdf_documents) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("❌ Failed to merge PDFs: {}", e);
            // Fallback to lopdf
            eprintln!("🔄 Trying fallback method with lopdf...");
            match merge_pdfs_with_lopdf(&pdf_documents) {
                Ok(data) => data,
                Err(e2) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to merge PDFs: {} (fallback also failed: {})", e, e2),
                    )
                        .into_response()
                }
            }
        }
    };

    // อัปโหลด merged PDF
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let filename = format!("merged_{}.pdf", Uuid::new_v4());
    let blob_path = format!("output/{}/{}", today, filename);

    let blob_client = container_client.blob_client(&blob_path);
    if let Err(e) = blob_client
        .put_block_blob(merged_pdf.clone())
        .content_type("application/pdf")
        .into_future()
        .await
    {
        eprintln!("upload error: {e:?}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Upload error").into_response();
    }

    let download_url = format!("{}/dl/{}", api_base(), encode(&blob_path));
    let size_kb = merged_pdf.len() as u64 / 1024;

    // ส่งผลลัพธ์
    let mut resp = Json(ConvertResp {
        ok: true,
        filename,
        size_kb,
        download_url,
        download_url_array: None,
        quota: QuotaPayload {
            plan: qr.plan.clone(),
            remaining_day: qr.remaining_day,
            remaining_month: qr.remaining_month,
        },
    })
    .into_response();
    add_quota_headers(&mut resp, &qr);
    resp
}

/// Merge PDFs using Python (pypdf) - รองรับภาษาไทยและ fonts ครบถ้วน
fn merge_pdfs_with_python(pdfs: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    use std::fs;
    use std::process::Command;

    let temp_dir = std::env::temp_dir();
    let output_path = temp_dir.join(format!("merged_{}.pdf", Uuid::new_v4()));

    // บันทึก PDF files เป็น temp files
    let mut temp_files = Vec::new();
    for (idx, (_name, data)) in pdfs.iter().enumerate() {
        let temp_file = temp_dir.join(format!("input_{}_{}.pdf", idx, Uuid::new_v4()));
        fs::write(&temp_file, data).map_err(|e| format!("Failed to write temp file: {}", e))?;
        temp_files.push(temp_file);
    }

    // สร้าง command arguments
    let mut args = vec![
        "merge_pdf.py".to_string(),
        output_path.to_str().unwrap().to_string(),
    ];
    for temp_file in &temp_files {
        args.push(temp_file.to_str().unwrap().to_string());
    }

    // เรียก Python script
    let output = Command::new("python3").args(&args).output().map_err(|e| {
        format!(
            "Failed to run Python merge script: {}. Please install python3 and pypdf.",
            e
        )
    })?;

    if !output.status.success() {
        // ลบ temp files
        for temp_file in temp_files {
            let _ = fs::remove_file(temp_file);
        }
        return Err(format!(
            "Python merge script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // อ่าน merged PDF
    let merged_data =
        fs::read(&output_path).map_err(|e| format!("Failed to read merged PDF: {}", e))?;

    // ลบ temp files
    for temp_file in temp_files {
        let _ = fs::remove_file(temp_file);
    }
    let _ = fs::remove_file(output_path);

    Ok(merged_data)
}

/// คัดลอก object และ dependencies ทั้งหมดแบบ recursive
fn copy_object_recursive(
    src_doc: &Document,
    src_id: (u32, u16),
    dest_doc: &mut Document,
    copied: &mut std::collections::HashMap<(u32, u16), (u32, u16)>,
) -> Result<(u32, u16), String> {
    // ถ้าคัดลอกไปแล้ว ใช้ ID เดิม
    if let Some(&dest_id) = copied.get(&src_id) {
        return Ok(dest_id);
    }

    // ดึง object จาก source document
    let obj = src_doc
        .get_object(src_id)
        .map_err(|e| format!("Failed to get object: {}", e))?
        .clone();

    // คัดลอก object แบบ recursive (สำหรับ references)
    let new_obj = match obj {
        lopdf::Object::Reference(ref_id) => {
            let new_ref_id = copy_object_recursive(src_doc, ref_id, dest_doc, copied)?;
            lopdf::Object::Reference(new_ref_id)
        }
        lopdf::Object::Dictionary(mut dict) => {
            // คัดลอกทุก value ใน dictionary (รวม nested references)
            for (_key, value) in dict.iter_mut() {
                if let lopdf::Object::Reference(ref_id) = value {
                    let new_ref_id = copy_object_recursive(src_doc, *ref_id, dest_doc, copied)?;
                    *value = lopdf::Object::Reference(new_ref_id);
                }
            }
            lopdf::Object::Dictionary(dict)
        }
        lopdf::Object::Array(mut arr) => {
            // คัดลอกทุก element ใน array
            for value in arr.iter_mut() {
                if let lopdf::Object::Reference(ref_id) = value {
                    let new_ref_id = copy_object_recursive(src_doc, *ref_id, dest_doc, copied)?;
                    *value = lopdf::Object::Reference(new_ref_id);
                }
            }
            lopdf::Object::Array(arr)
        }
        lopdf::Object::Stream(mut stream) => {
            // คัดลอก stream dictionary (สำคัญสำหรับ fonts!)
            for (_key, value) in stream.dict.iter_mut() {
                if let lopdf::Object::Reference(ref_id) = value {
                    let new_ref_id = copy_object_recursive(src_doc, *ref_id, dest_doc, copied)?;
                    *value = lopdf::Object::Reference(new_ref_id);
                }
            }
            lopdf::Object::Stream(stream)
        }
        _ => obj,
    };

    // เพิ่ม object เข้า destination document
    let dest_id = dest_doc.add_object(new_obj);
    copied.insert(src_id, dest_id);

    Ok(dest_id)
}

/// Merge multiple PDFs using lopdf
fn merge_pdfs_with_lopdf(pdfs: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    use lopdf::Object;

    // สร้าง document ใหม่
    let mut merged_doc = Document::with_version("1.5");
    let mut page_objects = Vec::new();

    eprintln!("🔄 Merging {} PDF files...", pdfs.len());

    // โหลดและรวม pages จากทุก PDF
    for (idx, (_filename, data)) in pdfs.iter().enumerate() {
        eprintln!("📄 Processing PDF {} of {}...", idx + 1, pdfs.len());

        let doc = Document::load_mem(data)
            .map_err(|e| format!("Failed to load PDF {}: {}", idx + 1, e))?;

        let pages = doc.get_pages();
        eprintln!("  📋 Found {} pages", pages.len());

        // สำหรับเก็บ object IDs ที่คัดลอกไปแล้ว
        let mut copied_objects = std::collections::HashMap::new();

        // เก็บ page object IDs จาก document นี้
        for (page_num, page_id) in pages.iter() {
            eprintln!(
                "    ➕ Adding page {} with all resources (fonts, images, etc.)",
                page_num
            );

            // คัดลอก page และ dependencies ทั้งหมดแบบ recursive
            let new_page_id =
                copy_object_recursive(&doc, *page_id, &mut merged_doc, &mut copied_objects)
                    .map_err(|e| format!("Failed to copy page {}: {}", page_num, e))?;

            // ลบ Parent reference (จะถูกตั้งค่าใหม่ภายหลัง)
            if let Ok(Object::Dictionary(ref mut page_dict)) =
                merged_doc.get_object_mut(new_page_id)
            {
                page_dict.remove(b"Parent");
            }

            page_objects.push(new_page_id);
        }
    }

    eprintln!("📚 Total pages in merged PDF: {}", page_objects.len());

    // สร้าง page tree
    let pages_dict = lopdf::Dictionary::from_iter(vec![
        ("Type", Object::Name(b"Pages".to_vec())),
        ("Count", Object::Integer(page_objects.len() as i64)),
        (
            "Kids",
            Object::Array(
                page_objects
                    .iter()
                    .map(|&id| Object::Reference(id))
                    .collect(),
            ),
        ),
    ]);

    let pages_id = merged_doc.add_object(Object::Dictionary(pages_dict));

    // อัพเดท parent reference ของแต่ละ page
    for &page_id in &page_objects {
        if let Ok(Object::Dictionary(ref mut page)) = merged_doc.get_object_mut(page_id) {
            page.set("Parent", Object::Reference(pages_id));
        }
    }

    // สร้าง catalog
    let catalog_dict = lopdf::Dictionary::from_iter(vec![
        ("Type", Object::Name(b"Catalog".to_vec())),
        ("Pages", Object::Reference(pages_id)),
    ]);

    let catalog_id = merged_doc.add_object(Object::Dictionary(catalog_dict));
    merged_doc
        .trailer
        .set("Root", Object::Reference(catalog_id));

    // แปลง merged document เป็น bytes
    let mut output = Vec::new();
    merged_doc
        .save_to(&mut output)
        .map_err(|e| format!("Failed to save merged PDF: {}", e))?;

    eprintln!(
        "✅ Merge completed! Output size: {} KB",
        output.len() / 1024
    );

    Ok(output)
}

// PDF conversion handler
async fn convert_pdf(
    State(st): State<AppState>,
    headers: HeaderMap,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Query(q): Query<ConvertQuery>,
    mut mp: Multipart,
) -> impl IntoResponse {
    // ✅ ใช้ตัวตนที่มีผล (user id หรือ ip)
    let (user_id, plan) = effective_user(&headers, Some(remote));

    // เตรียม Azure
    let (service, container) = match blob_service_from_env() {
        Ok(v) => v,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let container_client = service.container_client(&container);
    let _ = container_client
        .create()
        .public_access(PublicAccess::None)
        .into_future()
        .await;

    // 1) รับ/คุมพารามิเตอร์
    let max_upload_mb = q.max_upload_mb.unwrap_or(8);
    let max_upload_bytes_per_file: usize = (max_upload_mb as usize) * 1024 * 1024;

    // 2) อ่าน multipart: field ชื่อ "file"
    while let Ok(Some(field)) = mp.next_field().await {
        if field.name() == Some("file") {
            // ✅ ขอใช้โควตา 1 ภาพก่อนเริ่มทำ
            let qr = st.quota.try_consume(&user_id, 1, &plan);
            if !qr.allowed {
                let body = serde_json::json!({
                    "ok": false,
                    "error": qr.message.as_deref().unwrap_or("quota exceeded"),
                    "quota": {
                        "plan": qr.plan,
                        "remaining_day": qr.remaining_day,
                        "remaining_month": qr.remaining_month
                    }
                });
                let mut resp = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
                add_quota_headers(&mut resp, &qr);
                return resp;
            }

            // เช็กขนาดต่อไฟล์
            let data = match field.bytes().await {
                Ok(b) => {
                    if b.len() > max_upload_bytes_per_file {
                        return (
                            StatusCode::PAYLOAD_TOO_LARGE,
                            format!("file too large (max {}MB)", max_upload_mb),
                        )
                            .into_response();
                    }
                    b
                }
                Err(_) => return (StatusCode::BAD_REQUEST, "Invalid file").into_response(),
            };

            // ตรวจสอบว่าเป็น PDF หรือไม่
            if !is_pdf_format(&data) {
                return (StatusCode::BAD_REQUEST, "File is not a PDF").into_response();
            }

            // แปลง PDF เป็นรูปภาพทุกหน้า
            let temp_dir = std::env::temp_dir();
            let temp_pdf = temp_dir.join("temp.pdf");

            // บันทึก PDF ลงไฟล์ชั่วคราว
            if let Err(e) = std::fs::write(&temp_pdf, data) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to write temp PDF: {}", e),
                )
                    .into_response();
            }

            // เรียก pdftoppm เพื่อแปลง PDF เป็น JPEG ทุกหน้า
            let output = std::process::Command::new("pdftoppm")
                .arg("-jpeg")
                .arg("-r")
                .arg("150") // DPI
                .arg(&temp_pdf)
                .arg(&temp_dir.join("temp_output"))
                .output()
                .map_err(|e| {
                    format!(
                        "Failed to run pdftoppm: {}. Please install poppler-utils.",
                        e
                    )
                });

            let output = match output {
                Ok(output) => output,
                Err(e) => {
                    return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
                }
            };

            if !output.status.success() {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "pdftoppm failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ),
                )
                    .into_response();
            }

            // หาไฟล์ JPEG ทั้งหมดที่สร้างขึ้นและอัปโหลดทีละไฟล์
            let mut download_urls = Vec::new();
            let mut page_num = 1;
            let today = Utc::now().format("%Y-%m-%d").to_string();

            loop {
                let jpeg_file = temp_dir.join(format!("temp_output-{}.jpg", page_num));
                if jpeg_file.exists() {
                    let jpeg_data = match std::fs::read(&jpeg_file) {
                        Ok(data) => data,
                        Err(e) => {
                            eprintln!("Failed to read JPEG file: {}", e);
                            continue;
                        }
                    };

                    // อัปโหลดรูปภาพแต่ละหน้า
                    let filename = format!("page_{:03}_{}.jpg", page_num, Uuid::new_v4());
                    let blob_path = format!("output/{}/{}", today, filename);

                    let blob_client = container_client.blob_client(&blob_path);
                    if let Err(e) = blob_client
                        .put_block_blob(jpeg_data.clone())
                        .content_type("image/jpeg")
                        .into_future()
                        .await
                    {
                        eprintln!("upload error: {e:?}");
                        continue;
                    }

                    let download_url = format!("{}/dl/{}", api_base(), encode(&blob_path));
                    download_urls.push(download_url);
                    page_num += 1;
                } else {
                    break;
                }
            }

            if download_urls.is_empty() {
                return (StatusCode::INTERNAL_SERVER_ERROR, "No pages generated")
                    .into_response();
            }

            // ลบไฟล์ชั่วคราว
            let _ = std::fs::remove_file(&temp_pdf);
            for i in 1..page_num {
                let _ = std::fs::remove_file(temp_dir.join(format!("temp_output-{}.jpg", i)));
            }

            // ส่งผลลัพธ์เป็น array ของ URL
            let mut resp = Json(ConvertResp {
                ok: true,
                filename: format!("pdf_pages_{}.zip", Uuid::new_v4()), // ชื่อไฟล์สำหรับ frontend
                size_kb: 0,                                            // ไม่ใช้เพราะเป็น array
                download_url: download_urls[0].clone(),                // URL แรกเป็นหลัก
                download_url_array: Some(download_urls),               // array ของ URL ทั้งหมด
                quota: QuotaPayload {
                    plan: qr.plan.clone(),
                    remaining_day: qr.remaining_day,
                    remaining_month: qr.remaining_month,
                },
            })
            .into_response();

            add_quota_headers(&mut resp, &qr);
            return resp;
        }
    }

    (StatusCode::BAD_REQUEST, "No file received").into_response()
}

// PDF conversion handler สำหรับทุกหน้า
async fn convert_pdf_all(
    State(st): State<AppState>,
    headers: HeaderMap,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Query(q): Query<ConvertQuery>,
    mut mp: Multipart,
) -> impl IntoResponse {
    // ✅ ใช้ตัวตนที่มีผล (user id หรือ ip)
    let (user_id, plan) = effective_user(&headers, Some(remote));

    // เตรียม Azure
    let (service, container) = match blob_service_from_env() {
        Ok(v) => v,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let container_client = service.container_client(&container);
    let _ = container_client
        .create()
        .public_access(PublicAccess::None)
        .into_future()
        .await;

    // 1) รับ/คุมพารามิเตอร์
    let max_upload_mb = q.max_upload_mb.unwrap_or(8);
    let max_upload_bytes_per_file: usize = (max_upload_mb as usize) * 1024 * 1024;

    // สำหรับ PDF conversion ทุกหน้า

    // 2) อ่าน multipart: field ชื่อ "file"
    while let Ok(Some(field)) = mp.next_field().await {
        if field.name() == Some("file") {
            // ✅ ขอใช้โควตา 1 ภาพก่อนเริ่มทำ
            let qr = st.quota.try_consume(&user_id, 1, &plan);
            if !qr.allowed {
                let body = serde_json::json!({
                    "ok": false,
                    "error": qr.message.as_deref().unwrap_or("quota exceeded"),
                    "quota": {
                        "plan": qr.plan,
                        "remaining_day": qr.remaining_day,
                        "remaining_month": qr.remaining_month
                    }
                });
                let mut resp = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
                add_quota_headers(&mut resp, &qr);
                return resp;
            }

            // เช็กขนาดต่อไฟล์
            let data = match field.bytes().await {
                Ok(b) => {
                    if b.len() > max_upload_bytes_per_file {
                        return (
                            StatusCode::PAYLOAD_TOO_LARGE,
                            format!("file too large (max {}MB)", max_upload_mb),
                        )
                            .into_response();
                    }
                    b
                }
                Err(_) => return (StatusCode::BAD_REQUEST, "Invalid file").into_response(),
            };

            // ตรวจสอบว่าเป็น PDF หรือไม่
            if !is_pdf_format(&data) {
                return (StatusCode::BAD_REQUEST, "File is not a PDF").into_response();
            }

            // แปลง PDF เป็นรูปภาพทุกหน้า
            let temp_dir = std::env::temp_dir();
            let temp_pdf = temp_dir.join("temp.pdf");

            // บันทึก PDF ลงไฟล์ชั่วคราว
            if let Err(e) = std::fs::write(&temp_pdf, data) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to write temp PDF: {}", e),
                )
                    .into_response();
            }

            // เรียก pdftoppm เพื่อแปลง PDF เป็น JPEG ทุกหน้า
            let output = std::process::Command::new("pdftoppm")
                .arg("-jpeg")
                .arg("-r")
                .arg("150") // DPI
                .arg(&temp_pdf)
                .arg(&temp_dir.join("temp_output"))
                .output()
                .map_err(|e| {
                    format!(
                        "Failed to run pdftoppm: {}. Please install poppler-utils.",
                        e
                    )
                });

            let output = match output {
                Ok(output) => output,
                Err(e) => {
                    return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
                }
            };

            if !output.status.success() {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "pdftoppm failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ),
                )
                    .into_response();
            }

            // หาไฟล์ JPEG ทั้งหมดที่สร้างขึ้นและอัปโหลดทีละไฟล์
            let mut download_urls = Vec::new();
            let mut page_num = 1;
            let today = Utc::now().format("%Y-%m-%d").to_string();

            loop {
                let jpeg_file = temp_dir.join(format!("temp_output-{}.jpg", page_num));
                if jpeg_file.exists() {
                    let jpeg_data = match std::fs::read(&jpeg_file) {
                        Ok(data) => data,
                        Err(e) => {
                            eprintln!("Failed to read JPEG file: {}", e);
                            continue;
                        }
                    };

                    // อัปโหลดรูปภาพแต่ละหน้า
                    let filename = format!("page_{:03}_{}.jpg", page_num, Uuid::new_v4());
                    let blob_path = format!("output/{}/{}", today, filename);

                    let blob_client = container_client.blob_client(&blob_path);
                    if let Err(e) = blob_client
                        .put_block_blob(jpeg_data.clone())
                        .content_type("image/jpeg")
                        .into_future()
                        .await
                    {
                        eprintln!("upload error: {e:?}");
                        continue;
                    }

                    let download_url = format!("{}/dl/{}", api_base(), encode(&blob_path));
                    download_urls.push(download_url);
                    page_num += 1;
                } else {
                    break;
                }
            }

            if download_urls.is_empty() {
                return (StatusCode::INTERNAL_SERVER_ERROR, "No pages generated").into_response();
            }

            // ลบไฟล์ชั่วคราว
            let _ = std::fs::remove_file(&temp_pdf);
            for i in 1..page_num {
                let _ = std::fs::remove_file(temp_dir.join(format!("temp_output-{}.jpg", i)));
            }

            // ส่งผลลัพธ์เป็น array ของ URL
            let mut resp = Json(ConvertResp {
                ok: true,
                filename: format!("pdf_pages_{}.zip", Uuid::new_v4()), // ชื่อไฟล์สำหรับ frontend
                size_kb: 0,                                            // ไม่ใช้เพราะเป็น array
                download_url: download_urls[0].clone(),                // URL แรกเป็นหลัก
                download_url_array: Some(download_urls),               // array ของ URL ทั้งหมด
                quota: QuotaPayload {
                    plan: qr.plan.clone(),
                    remaining_day: qr.remaining_day,
                    remaining_month: qr.remaining_month,
                },
            })
            .into_response();

            add_quota_headers(&mut resp, &qr);
            return resp;
        }
    }

    (StatusCode::BAD_REQUEST, "No file received").into_response()
}

// โหลดไฟล์จาก Blob แล้วสตรีมกลับ (ซ่อน URL จริง)
async fn download_from_blob(Path(path): Path<String>) -> impl IntoResponse {
    let (service, container) = match blob_service_from_env() {
        Ok(v) => v,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let container_client = service.container_client(&container);
    let blob_client = container_client.blob_client(&path);

    match blob_client.get_content().await {
        Ok(bytes) => {
            let mut headers = HeaderMap::new();
            let ct = guess_content_type_by_ext(&path);
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_static(ct),
            );
            (headers, bytes).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "File not found").into_response(),
    }
}
