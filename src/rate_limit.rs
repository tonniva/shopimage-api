use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::ConnectInfo,
    extract::{Request, State}, // ⬅️ ใช้ Request alias ของ Axum
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};

use dashmap::DashMap;
use serde_json::json;

#[derive(Clone)]
pub struct RateLimitState {
    buckets: std::sync::Arc<DashMap<String, Bucket>>,
    capacity: f32,       // โควตาสูงสุด/นาที (เช่น 60)
    refill_per_sec: f32, // เติม token ต่อวินาที (capacity/60)
    lock_enabled: bool,  // เปิด lock หรือไม่
    lock_secs: u64,      // ระยะเวลา lock (วินาที)
}

#[derive(Debug)]
struct Bucket {
    tokens: f32,
    last: Instant,
    lock_until: Option<Instant>,
}

impl RateLimitState {
    #[allow(dead_code)]
    pub fn new(limit_per_minute: u32) -> Self {
        Self::new_with_lock(limit_per_minute, false, 0)
    }

    pub fn new_with_lock(limit_per_minute: u32, lock_enabled: bool, lock_secs: u64) -> Self {
        let capacity = limit_per_minute as f32;
        let refill_per_sec = capacity / 60.0;
        Self {
            buckets: std::sync::Arc::new(DashMap::new()),
            capacity,
            refill_per_sec,
            lock_enabled,
            lock_secs,
        }
    }

    fn take(&self, key: &str) -> TakeResult {
        let now = Instant::now();
        let mut entry = self
            .buckets
            .entry(key.to_string())
            .or_insert_with(|| Bucket {
                tokens: self.capacity,
                last: now,
                lock_until: None,
            });

        // ถ้าถูกล็อกอยู่ → ปฏิเสธทันที
        if let Some(t) = entry.lock_until {
            if t > now {
                let remain = (t - now).as_secs().max(1);
                return TakeResult::Locked {
                    retry_after: remain,
                    limit: self.capacity as u32,
                    remaining: 0,
                };
            } else {
                entry.lock_until = None; // หมดเวลาล็อกแล้ว
            }
        }

        // เติม token จากเวลาที่ผ่านไป
        let elapsed = now.duration_since(entry.last).as_secs_f32();
        entry.tokens = (entry.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        entry.last = now;

        // ขอใช้ 1 token
        if entry.tokens >= 1.0 {
            entry.tokens -= 1.0;
            TakeResult::Ok {
                limit: self.capacity as u32,
                remaining: entry.tokens.floor() as u32,
            }
        } else {
            // โควตาไม่พอ → ถ้าเปิด lock ให้ล็อกทันที
            if self.lock_enabled && self.lock_secs > 0 {
                entry.lock_until = Some(now + Duration::from_secs(self.lock_secs));
                TakeResult::Locked {
                    retry_after: self.lock_secs,
                    limit: self.capacity as u32,
                    remaining: 0,
                }
            } else {
                // ไม่มี lock → แจ้งให้รอจน token กลับมา >= 1
                let need = 1.0 - entry.tokens.max(0.0);
                let secs = (need / self.refill_per_sec).ceil().max(1.0) as u64;
                TakeResult::Limited {
                    retry_after: secs,
                    limit: self.capacity as u32,
                    remaining: entry.tokens.floor() as u32,
                }
            }
        }
    }
}

enum TakeResult {
    Ok {
        limit: u32,
        remaining: u32,
    },
    Limited {
        retry_after: u64,
        limit: u32,
        remaining: u32,
    },
    Locked {
        retry_after: u64,
        limit: u32,
        remaining: u32,
    },
}

pub async fn rate_limit(
    State(state): State<RateLimitState>, // ⬅️ ดึง state ด้วย pattern extractor
    req: Request,                    // ⬅️ ไม่มี generic <Body> แล้ว
    next: Next,                          // ⬅️ ไม่มี generic <Body> แล้ว
) -> Response {
    let ip = client_ip(&req).unwrap_or_else(|| "unknown".to_string());
    match state.take(&ip) {
        TakeResult::Ok { limit, remaining } => {
            let mut res = next.run(req).await;
            res.headers_mut().insert(
                "x-ratelimit-limit",
                HeaderValue::from_str(&limit.to_string()).unwrap(),
            );
            res.headers_mut().insert(
                "x-ratelimit-remaining",
                HeaderValue::from_str(&remaining.to_string()).unwrap(),
            );
            res
        }
        TakeResult::Limited {
            retry_after,
            limit,
            remaining,
        }
        | TakeResult::Locked {
            retry_after,
            limit,
            remaining,
        } => {
            let body = serde_json::to_vec(&json!({
                "ok": false,
                "error": "Too many requests, please try again later"
            }))
            .unwrap();

            Response::builder()
                .status(StatusCode::TOO_MANY_REQUESTS)
                .header("content-type", "application/json; charset=utf-8")
                .header(
                    "retry-after",
                    HeaderValue::from_str(&retry_after.to_string()).unwrap(),
                )
                .header(
                    "x-ratelimit-limit",
                    HeaderValue::from_str(&limit.to_string()).unwrap(),
                )
                .header(
                    "x-ratelimit-remaining",
                    HeaderValue::from_str(&remaining.to_string()).unwrap(),
                )
                .body(Body::from(body))
                .unwrap()
        }
    }
}
fn client_ip<B>(req: &Request<B>) -> Option<String> {
    if let Some(v) = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|h| h.to_str().ok())
    {
        if let Some(first) = v.split(',').next() {
            let s = first.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    if let Some(v) = req.headers().get("x-real-ip").and_then(|h| h.to_str().ok()) {
        let s = v.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().to_string())
}
