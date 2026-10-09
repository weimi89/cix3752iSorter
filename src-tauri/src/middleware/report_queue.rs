//! `POST /api/report` 佇列：先落 `report_queue` 表再由 worker 送，重啟不丟、失敗指數退避。
//!
//! 4xx（含 422「找不到 response_id」）視為永久失敗，不再重試；其餘退避重試到上限後標失敗。

use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::Middleware;
use crate::db::{DbPool, now_ms};
use crate::event_bus;
use crate::event_log::{self, Level};

const MAX_RETRY: i64 = 10;
/// 退避上限（毫秒）：10 分鐘
const MAX_BACKOFF_MS: i64 = 600_000;

pub fn backoff_ms(retry_count: i64) -> i64 {
    // 5s, 10s, 20s, … 封頂 10 分鐘
    (5_000i64.saturating_mul(1i64 << retry_count.clamp(0, 20))).min(MAX_BACKOFF_MS)
}

/// 認領一筆到期的回報。入列時查不到的包裹編號由包裹寫入端在寫回報編號時補上（`tracker::store`）
async fn claim(db: &DbPool, now: i64) -> Result<Option<(i64, i64, i64)>, sqlx::Error> {
    sqlx::query_as(
        "UPDATE report_queue SET status = 'sending'
         WHERE id = (SELECT id FROM report_queue WHERE status = 'pending' AND next_retry_ms <= ?1 ORDER BY id LIMIT 1)
         RETURNING id, response_id, retry_count",
    )
    .bind(now)
    .fetch_optional(db)
    .await
}

#[derive(Clone)]
pub struct ReportQueue {
    db: DbPool,
}

impl ReportQueue {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    /// 入列。同一 `response_id` 只留一筆（中介機端也保證唯一）。
    /// 資料庫忙（等滿 busy_timeout 仍拿不到寫鎖）時退避重試：這裡失敗就沒有人會再送這筆回報
    pub async fn enqueue(&self, parcel_ulid: &str, response_id: i64) -> Result<(), sqlx::Error> {
        crate::db::retry_busy("回報入列", || self.try_enqueue(parcel_ulid, response_id)).await
    }

    /// 包裹列由狀態機的寫入 task 非同步插入，忙的時候會比這裡晚到：查不到就先留空包裹編號，
    /// 不可寫成 `INSERT … SELECT … FROM parcels`（查不到時插入 0 筆、不報錯，回報就無聲消失）
    async fn try_enqueue(&self, parcel_ulid: &str, response_id: i64) -> Result<(), sqlx::Error> {
        let now = now_ms();
        let payload = serde_json::json!({ "response_id": response_id }).to_string();
        sqlx::query(
            "INSERT INTO report_queue (parcel_id, response_id, payload, status, next_retry_ms, created_ms)
             SELECT (SELECT id FROM parcels WHERE ulid = ?1), ?2, ?3, 'pending', 0, ?4
              WHERE NOT EXISTS (SELECT 1 FROM report_queue WHERE response_id = ?2)",
        )
        .bind(parcel_ulid)
        .bind(response_id)
        .bind(payload)
        .bind(now)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// 手動重送（網頁）
    pub async fn retry(&self, id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE report_queue SET status = 'pending', next_retry_ms = 0, retry_count = 0, last_error = NULL WHERE id = ?")
            .bind(id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    pub fn start_worker(&self, mw: Middleware, cancel: CancellationToken) {
        let db = self.db.clone();
        crate::supervise::spawn("回報佇列", async move {
            // 上次程式中斷時卡在 sending 的，重新排隊
            let _ = sqlx::query("UPDATE report_queue SET status = 'pending' WHERE status = 'sending'").execute(&db).await;
            let mut tick = tokio::time::interval(Duration::from_millis(500));
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tick.tick() => {}
                }
                loop {
                    let now = now_ms();
                    let Some((id, response_id, retry_count)) = claim(&db, now).await.unwrap_or(None) else { break };

                    match mw.report(response_id).await {
                        Ok(()) => {
                            let _ = sqlx::query("UPDATE report_queue SET status = 'success', finished_ms = ?, last_error = NULL WHERE id = ?")
                                .bind(now_ms())
                                .bind(id)
                                .execute(&db)
                                .await;
                            event_bus::emit("report-queue", serde_json::json!({ "id": id, "status": "success" }));
                        }
                        Err(e) => {
                            let permanent = e.is_permanent() || retry_count + 1 >= MAX_RETRY;
                            let msg = e.to_string();
                            if permanent {
                                let _ = sqlx::query("UPDATE report_queue SET status = 'failed', finished_ms = ?, last_error = ?, retry_count = retry_count + 1 WHERE id = ?")
                                    .bind(now_ms())
                                    .bind(&msg)
                                    .bind(id)
                                    .execute(&db)
                                    .await;
                                event_log::log(&db, Level::Error, "middleware", "report_failed", format!("回報 response_id={response_id} 失敗（不再重試）: {msg}"));
                            } else {
                                let next = now_ms() + backoff_ms(retry_count);
                                let _ = sqlx::query("UPDATE report_queue SET status = 'pending', next_retry_ms = ?, last_error = ?, retry_count = retry_count + 1 WHERE id = ?")
                                    .bind(next)
                                    .bind(&msg)
                                    .bind(id)
                                    .execute(&db)
                                    .await;
                                tracing::warn!(response_id, retry = retry_count + 1, "回報失敗，稍後重試: {msg}");
                            }
                            event_bus::emit("report-queue", serde_json::json!({ "id": id, "status": if permanent { "failed" } else { "pending" }, "error": msg }));
                            // 連線層失敗時整批都會失敗，不要一口氣把佇列掃完
                            if !permanent {
                                break;
                            }
                        }
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 退避指數成長且封頂() {
        assert_eq!(backoff_ms(0), 5_000);
        assert_eq!(backoff_ms(1), 10_000);
        assert_eq!(backoff_ms(3), 40_000);
        assert_eq!(backoff_ms(9), 600_000);
        assert_eq!(backoff_ms(30), 600_000);
    }

    async fn test_db() -> DbPool {
        let dir = std::env::temp_dir().join(format!("rq-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        crate::db::init(&dir).await.unwrap()
    }

    async fn rows(db: &DbPool) -> Vec<(Option<i64>, i64, String)> {
        sqlx::query_as("SELECT parcel_id, response_id, status FROM report_queue ORDER BY id").fetch_all(db).await.unwrap()
    }

    #[tokio::test]
    async fn 包裹列還沒寫進資料庫時入列_回報照樣建立_不可無聲遺失() {
        // 包裹列由寫入 task 非同步插入，忙的時候會晚到；回報送出只需要 response_id
        let db = test_db().await;
        let q = ReportQueue::new(db.clone());
        q.enqueue("NOT-YET-INSERTED", 17105579).await.unwrap();
        assert_eq!(rows(&db).await, vec![(None, 17105579, "pending".to_string())]);
    }

    #[tokio::test]
    async fn 包裹列已在時帶上包裹編號_同一回報重複入列只留一筆() {
        let db = test_db().await;
        let pid: i64 = sqlx::query_scalar("INSERT INTO parcels (ulid, barcode, status, started_at, started_ms, updated_ms) VALUES ('U1', 'A1', 2, '2026-10-08 18:10:39.707', 1, 1) RETURNING id")
            .fetch_one(&db).await.unwrap();
        let q = ReportQueue::new(db.clone());
        q.enqueue("U1", 42).await.unwrap();
        q.enqueue("U1", 42).await.unwrap();
        assert_eq!(rows(&db).await, vec![(Some(pid), 42, "pending".to_string())]);
    }


    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn 資料庫被握住超過等待上限時_入列會重試_鎖放開後成功() {
        let db = test_db().await;
        let mut holder = db.acquire().await.unwrap();
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *holder).await.unwrap();
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(6_000)).await;
            sqlx::query("COMMIT").execute(&mut *holder).await.unwrap();
        });
        let q = ReportQueue::new(db.clone());
        let r = q.enqueue("U-LOCKED", 7).await;
        release.await.unwrap();
        assert!(r.is_ok(), "鎖放開後應成功：{r:?}");
        assert_eq!(rows(&db).await, vec![(None, 7, "pending".to_string())]);
    }


    #[tokio::test]
    async fn 認領只取到期的待送回報_已認領的不再被取() {
        let db = test_db().await;
        let q = ReportQueue::new(db.clone());
        q.enqueue("ANY", 99).await.unwrap();
        let got = claim(&db, now_ms()).await.unwrap();
        assert!(got.is_some_and(|(_, rid, _)| rid == 99));
        assert_eq!(rows(&db).await, vec![(None, 99, "sending".to_string())]);
        assert!(claim(&db, now_ms()).await.unwrap().is_none(), "已認領的不會再被認領");
    }


    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn 關閉時資料庫忙碌_結束流程要等入列重試完成_不可在重試中途結束() {
        let db = test_db().await;
        let mut holder = db.acquire().await.unwrap();
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *holder).await.unwrap();
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(7_000)).await;
            sqlx::query("COMMIT").execute(&mut *holder).await.unwrap();
        });
        // 跟正式的結束流程一樣：入列登記在 tasks，關閉時用 DRAIN_LIMIT 等
        let tasks = tokio_util::task::TaskTracker::new();
        let q = ReportQueue::new(db.clone());
        tasks.spawn(async move { q.enqueue("U-SHUTDOWN", 8).await });
        let drained = crate::supervise::drain(&tasks, crate::supervise::DRAIN_LIMIT).await;
        release.await.unwrap();
        assert!(drained, "時限內要等到入列完成");
        assert_eq!(rows(&db).await, vec![(None, 8, "pending".to_string())]);
    }
}
