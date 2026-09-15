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

#[derive(Clone)]
pub struct ReportQueue {
    db: DbPool,
}

impl ReportQueue {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    /// 入列。同一 `response_id` 只留一筆（中介機端也保證唯一）。
    pub async fn enqueue(&self, parcel_ulid: &str, response_id: i64) -> Result<(), sqlx::Error> {
        let now = now_ms();
        let payload = serde_json::json!({ "response_id": response_id }).to_string();
        sqlx::query(
            "INSERT INTO report_queue (parcel_id, response_id, payload, status, next_retry_ms, created_ms)
             SELECT id, ?2, ?3, 'pending', 0, ?4 FROM parcels WHERE ulid = ?1
             ON CONFLICT DO NOTHING",
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
        tokio::spawn(async move {
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
                    // 認領一筆到期的
                    let row: Option<(i64, i64, i64)> = sqlx::query_as(
                        "UPDATE report_queue SET status = 'sending'
                         WHERE id = (SELECT id FROM report_queue WHERE status = 'pending' AND next_retry_ms <= ?1 ORDER BY id LIMIT 1)
                         RETURNING id, response_id, retry_count",
                    )
                    .bind(now)
                    .fetch_optional(&db)
                    .await
                    .unwrap_or(None);
                    let Some((id, response_id, retry_count)) = row else { break };

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
}
