use chrono::{DateTime, Datelike, Utc};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
}; // ⬅️ เพิ่มบรรทัดนี้
#[derive(Clone, Debug)]
pub struct PlanQuota {
    pub daily: Option<u64>, // None = ไม่จำกัดรายวัน
    pub monthly: u64,       // รายเดือนต้องมี
}


fn plan_quota(plan: &str) -> PlanQuota {
    match plan {
        "pro" => PlanQuota {
            daily: None,
            monthly: 5_000,
        },
        "business" => PlanQuota {
            daily: None,
            monthly: 10_000,
        },
        _ => PlanQuota {
            daily: Some(100),
            monthly: 1_000,
        }, // default = free
    }
}

#[derive(Clone, Debug)]
struct Usage {
    day_key: (i32, u32, u32), // (year, month, day) UTC
    day_count: u64,

    month_key: (i32, u32), // (year, month) UTC
    month_count: u64,
}

impl Usage {
    fn new(now: DateTime<Utc>) -> Self {
        Usage {
            day_key: (now.year(), now.month(), now.day()),
            day_count: 0,
            month_key: (now.year(), now.month()),
            month_count: 0,
        }
    }
    fn rotate_if_needed(&mut self, now: DateTime<Utc>) {
        let dkey = (now.year(), now.month(), now.day());
        if dkey != self.day_key {
            self.day_key = dkey;
            self.day_count = 0;
        }
        let mkey = (now.year(), now.month());
        if mkey != self.month_key {
            self.month_key = mkey;
            self.month_count = 0;
        }
    }
}

#[derive(Clone)]
pub struct Quota {
    inner: Arc<Mutex<HashMap<String, Usage>>>, // user_id -> usage
}

#[derive(Debug, Serialize, Clone)] // ⬅️ เพิ่ม Serialize (+ Clone เผื่อใช้ซ้ำ)
pub struct QuotaResult {
    pub allowed: bool,
    pub remaining_day: Option<u64>, // None = ไม่จำกัด
    pub remaining_month: u64,
    pub message: Option<String>,
    pub plan: String,
}

impl Quota {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// ตรวจและ "จองโควตา" จำนวน `amount` สำหรับ user_id + plan
    /// - จะอัปเดต counter (ถ้าพอ) แล้วคืนค่า remaining
    pub fn try_consume(&self, user_id: &str, amount: u64, plan: &str) -> QuotaResult {
        let now = Utc::now();
        let pq = plan_quota(plan);
        let mut map = self.inner.lock().unwrap();

        let usage = map
            .entry(user_id.to_string())
            .or_insert_with(|| Usage::new(now));

        usage.rotate_if_needed(now);

        // เช็คเดือน
        if usage.month_count + amount > pq.monthly {
            return QuotaResult {
                allowed: false,
                remaining_day: pq.daily.map(|d| d.saturating_sub(usage.day_count)),
                remaining_month: pq.monthly.saturating_sub(usage.month_count),
                message: Some(format!(
                    "Monthly quota exceeded (limit {} images)",
                    pq.monthly
                )),
                plan: plan.to_string(),
            };
        }

        // เช็ครายวัน (ถ้ามี)
        if let Some(daily_lim) = pq.daily {
            if usage.day_count + amount > daily_lim {
                return QuotaResult {
                    allowed: false,
                    remaining_day: Some(daily_lim.saturating_sub(usage.day_count)),
                    remaining_month: pq.monthly.saturating_sub(usage.month_count),
                    message: Some(format!("Daily quota exceeded (limit {} images)", daily_lim)),
                    plan: plan.to_string(),
                };
            }
        }

        // ผ่าน → หักโควตา
        usage.day_count += amount;
        usage.month_count += amount;

        QuotaResult {
            allowed: true,
            remaining_day: pq.daily.map(|d| d.saturating_sub(usage.day_count)),
            remaining_month: pq.monthly.saturating_sub(usage.month_count),
            message: None,
            plan: plan.to_string(),
        }
    }
}
