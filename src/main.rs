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
    quota: QuotaPayload, // ส่งเสมอ
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
    quota: Arc<quota::Quota>, // โควตาต่อ user/ip
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

    // ✅ สร้าง AppState (Quota memory)
    let state = AppState {
        quota: Arc::new(quota::Quota::new()),
    };

    // เส้นที่ต้อง rate-limit (ต่อ IP)
    let protected = Router::new()
        .route("/api/convert", post(convert))
        .route("/api/convert-batch", post(convert_batch::convert_batch))
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

    let mut max_bytes = q.max_kb.unwrap_or(2048) * 1024; // default 2MB
    let output_format = q.format.as_deref().unwrap_or("webp");

    let mut target_w = parse_u32_opt(q.target_w.as_deref()).map(sane_dim);
    let mut target_h = parse_u32_opt(q.target_h.as_deref()).map(sane_dim);

    if let Some(preset) = q.platform.as_deref().map(presets::preset) {
        max_bytes = q.max_kb.map(|v| v * 1024).unwrap_or(preset.max_bytes);
        if target_w.is_none() {
            target_w = preset.target_w;
        }
        if target_h.is_none() {
            target_h = preset.target_h;
        }
    }

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

            // โหลดรูป
            let mut img = match image::load_from_memory(&data) {
                Ok(i) => i,
                Err(_) => {
                    return (StatusCode::UNSUPPORTED_MEDIA_TYPE, "Unsupported image")
                        .into_response()
                }
            };

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
