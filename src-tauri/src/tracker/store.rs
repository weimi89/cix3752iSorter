//! 狀態機的 DB 寫入端：獨立 task 依序寫，狀態機本身不等 I/O。
//!
//! 寫入失敗一律記 event_log 並印錯誤，不吞掉；狀態機的記憶體狀態不受影響。

use std::collections::HashMap;

use tokio::sync::mpsc;

use crate::db::{DbPool, local_ts, now_ms};
use crate::event_log::{self, Level};

use super::parcel::Parcel;

pub enum StoreOp {
    Insert(Parcel),
    Update(Parcel),
    Event { ulid: String, ts_ms: i64, source: &'static str, kind: String, raw: Option<String> },
    /// 終態已寫入，之後不會再有這件的更新；釋放 ulid→id 對應
    Forget(String),
    /// 終態件計入當日統計
    Daily(Parcel),
    /// 一次堵塞開始（統計用）
    Jam { ts_ms: i64, cart: u32, pos: i32, ulid: Option<String>, barcode: Option<String>, chute_code: Option<String> },
}

#[derive(Clone)]
pub struct Store {
    tx: mpsc::Sender<StoreOp>,
}

impl Store {
    pub fn spawn(db: DbPool) -> Self {
        let (tx, rx) = mpsc::channel(8192);
        tokio::spawn(run(db, rx));
        Self { tx }
    }

    pub fn send(&self, op: StoreOp) {
        if let Err(e) = self.tx.try_send(op) {
            tracing::error!("store 佇列滿，寫入丟失: {e}");
        }
    }

    pub fn insert(&self, p: &Parcel) {
        self.send(StoreOp::Insert(p.clone()));
    }

    pub fn update(&self, p: &Parcel) {
        self.send(StoreOp::Update(p.clone()));
    }

    pub fn event(&self, p: &Parcel, ts_ms: i64, source: &'static str, kind: impl Into<String>, raw: Option<String>) {
        self.send(StoreOp::Event { ulid: p.ulid.clone(), ts_ms, source, kind: kind.into(), raw });
    }

    pub fn forget(&self, p: &Parcel) {
        self.send(StoreOp::Forget(p.ulid.clone()));
    }
}

/// 啟動時把上次沒收尾的在途件標成「失去追蹤」：重啟後分揀機已被重置，實體狀態無從得知。
pub async fn close_orphans(db: &DbPool) -> anyhow::Result<u64> {
    let now = now_ms();
    let r = sqlx::query(
        "UPDATE parcels SET status = 4, ended_ms = ?1, travel_ms = ?1 - started_ms, updated_ms = ?1, chute_source = CASE WHEN chute_source = 'pending' THEN 'timeout' ELSE chute_source END WHERE ended_ms IS NULL",
    )
    .bind(now)
    .execute(db)
    .await?;
    Ok(r.rows_affected())
}

async fn run(db: DbPool, mut rx: mpsc::Receiver<StoreOp>) {
    let mut ids: HashMap<String, i64> = HashMap::new();
    while let Some(op) = rx.recv().await {
        match op {
            StoreOp::Insert(p) => match insert(&db, &p).await {
                Ok(id) => {
                    ids.insert(p.ulid.clone(), id);
                }
                Err(e) => event_log::log(&db, Level::Error, "tracker", "db_insert", format!("包裹 {} 寫入失敗: {e}", p.ulid)),
            },
            StoreOp::Update(p) => {
                if let Err(e) = update(&db, &p).await {
                    event_log::log(&db, Level::Error, "tracker", "db_update", format!("包裹 {} 更新失敗: {e}", p.ulid));
                }
            }
            StoreOp::Event { ulid, ts_ms, source, kind, raw } => {
                let Some(&id) = ids.get(&ulid) else {
                    tracing::warn!(%ulid, %kind, "事件對應不到包裹 id（插入尚未成功？）");
                    continue;
                };
                if let Err(e) = sqlx::query("INSERT INTO parcel_events (parcel_id, ts_ms, source, kind, raw) VALUES (?, ?, ?, ?, ?)")
                    .bind(id)
                    .bind(ts_ms)
                    .bind(source)
                    .bind(&kind)
                    .bind(&raw)
                    .execute(&db)
                    .await
                {
                    tracing::error!(%ulid, "parcel_events 寫入失敗: {e}");
                }
            }
            StoreOp::Forget(ulid) => {
                ids.remove(&ulid);
            }
            StoreOp::Daily(p) => {
                if let Err(e) = daily(&db, &p).await {
                    tracing::error!("daily_stats 更新失敗: {e}");
                }
            }
            StoreOp::Jam { ts_ms, cart, pos, ulid, barcode, chute_code } => {
                let parcel_id = ulid.as_ref().and_then(|u| ids.get(u).copied());
                if let Err(e) = sqlx::query(
                    "INSERT INTO jam_events (ts_ms, created_at, cart, pos, module, parcel_id, barcode, chute_code) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(ts_ms)
                .bind(local_ts(ts_ms))
                .bind(cart as i64)
                .bind(pos)
                .bind(pos / 10 + 1)
                .bind(parcel_id)
                .bind(&barcode)
                .bind(&chute_code)
                .execute(&db)
                .await
                {
                    tracing::error!("jam_events 寫入失敗: {e}");
                }
            }
        }
    }
}

async fn insert(db: &DbPool, p: &Parcel) -> Result<i64, sqlx::Error> {
    let now = now_ms();
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO parcels (ulid, barcode, chute_source, status, belt_slot, started_at, started_ms, updated_ms)
         VALUES (?, ?, 'pending', ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&p.ulid)
    .bind(p.barcode_or_noread())
    .bind(p.status.code())
    .bind(p.slot as i64)
    .bind(local_ts(p.p_ms))
    .bind(p.p_ms)
    .bind(now)
    .fetch_one(db)
    .await?;
    Ok(row.0)
}

async fn update(db: &DbPool, p: &Parcel) -> Result<(), sqlx::Error> {
    let now = now_ms();
    let (code, cid, source, response_id, reason) = match &p.chute {
        Some(c) => (Some(c.code.clone()), Some(c.cid.0 as i64), serde_json::to_value(c.source).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "pending".into()), c.response_id, c.reason.clone()),
        None => (None, None, "pending".to_string(), None, None),
    };
    sqlx::query(
        "UPDATE parcels SET barcode = ?, chute_code = ?, chute_cid = ?, chute_source = ?, chute_reason = ?, status = ?, cart = ?,
            ir_length = ?, gap = ?, block_pos = ?, lost_pos = ?, response_id = ?, ended_ms = ?, travel_ms = ?, updated_ms = ?
         WHERE ulid = ?",
    )
    .bind(p.barcode_or_noread())
    .bind(code)
    .bind(cid)
    .bind(source)
    .bind(reason)
    .bind(p.status.code())
    .bind(p.cart.map(|c| c as i64))
    .bind(p.ir_length)
    .bind(p.gap)
    .bind(p.block_pos)
    .bind(p.lost_pos)
    .bind(response_id)
    .bind(p.ended_ms)
    .bind(p.ended_ms.map(|e| e - p.p_ms))
    .bind(now)
    .bind(&p.ulid)
    .execute(db)
    .await?;
    Ok(())
}

async fn daily(db: &DbPool, p: &Parcel) -> Result<(), sqlx::Error> {
    use super::parcel::{ChuteSource, Status};
    let day = local_ts(p.p_ms)[..10].to_string();
    let done = (p.status == Status::Done) as i64;
    let noread = (p.barcode.is_none() || p.barcode.as_deref() == Some(crate::device::camera::NO_READ)) as i64;
    let defaulted = p.chute.as_ref().is_some_and(|c| !matches!(c.source, ChuteSource::Api | ChuteSource::Manual)) as i64;
    let abnormal = (p.status != Status::Done) as i64;
    sqlx::query(
        "INSERT INTO daily_stats (day, total, done, noread, defaulted, abnormal) VALUES (?1, 1, ?2, ?3, ?4, ?5)
         ON CONFLICT(day) DO UPDATE SET total = total + 1, done = done + ?2, noread = noread + ?3, defaulted = defaulted + ?4, abnormal = abnormal + ?5",
    )
    .bind(day)
    .bind(done)
    .bind(noread)
    .bind(defaulted)
    .bind(abnormal)
    .execute(db)
    .await?;
    Ok(())
}
