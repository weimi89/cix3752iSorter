//! 資料保留：超過 `general.retention_days` 的包裹、事件、列印任務、回報紀錄、系統事件一併清掉，
//! 對應舊系統啟動時的 `DELETE … endTs < N 天前`，但改以「包裹上線時間」為準——
//! 舊版用結束時間，取消／異常件的結束時間是 0，一啟動就被整批誤刪（已驗證 472 筆）。
//!
//! 啟動時清一次，之後每小時一次；`retention_days = 0` 不清。

use std::path::Path;
use std::time::Duration;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::db::DbPool;
use crate::event_log::{self, Level};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Purged {
    pub parcels: u64,
    pub print_jobs: u64,
    pub reports: u64,
    pub events: u64,
}

impl Purged {
    pub fn total(&self) -> u64 {
        self.parcels + self.print_jobs + self.reports + self.events
    }
}

/// 刪掉 `days` 天前（以本機日期 00:00 為界）的資料；回傳各表刪除筆數
pub async fn purge(db: &DbPool, days: u32) -> Result<Purged, sqlx::Error> {
    if days == 0 {
        return Ok(Purged::default());
    }
    let cutoff_day = (chrono::Local::now() - chrono::Duration::days(days as i64)).format("%Y-%m-%d").to_string();
    let cutoff_ts = format!("{cutoff_day} 00:00:00");
    let cutoff_ms = chrono::NaiveDate::parse_from_str(&cutoff_day, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|dt| dt.and_local_timezone(chrono::Local).single())
        .map(|t| t.timestamp_millis())
        .unwrap_or(0);

    // 只清已結束的列印任務，點陣檔一起清（印失敗的任務檔案還留著）；還在等印表機的不動
    let spools: Vec<(String,)> = sqlx::query_as("SELECT tspl_path FROM print_jobs WHERE created_ms < ? AND status IN ('done', 'failed')").bind(cutoff_ms).fetch_all(db).await?;
    for (path,) in &spools {
        let _ = tokio::fs::remove_file(Path::new(path)).await;
    }
    let print_jobs = sqlx::query("DELETE FROM print_jobs WHERE created_ms < ? AND status IN ('done', 'failed')").bind(cutoff_ms).execute(db).await?.rows_affected();
    // 還沒送出去的回報不刪，重啟後仍要續送
    let reports = sqlx::query("DELETE FROM report_queue WHERE created_ms < ? AND status IN ('success', 'failed')").bind(cutoff_ms).execute(db).await?.rows_affected();
    // parcel_events 隨 parcels 級聯刪除
    let parcels = sqlx::query("DELETE FROM parcels WHERE started_at < ?").bind(&cutoff_ts).execute(db).await?.rows_affected();
    let events = sqlx::query("DELETE FROM event_log WHERE created_at < ?").bind(&cutoff_ts).execute(db).await?.rows_affected();
    // 卡件事件跟包裹同一個保留期
    sqlx::query("DELETE FROM jam_events WHERE created_at < ?").bind(&cutoff_ts).execute(db).await?;
    Ok(Purged { parcels, print_jobs, reports, events })
}

/// 啟動清一次，之後每小時；天數跟著設定即時變。照片另有自己的保留天數（`camera_ftp.retention_days`）
pub fn start(db: DbPool, data_dir: std::path::PathBuf, cfg: watch::Receiver<AppConfig>, cancel: CancellationToken) {
    tokio::spawn(async move {
        loop {
            let (days, image_days, images_dir) = {
                let c = cfg.borrow();
                (c.general.retention_days, c.camera_ftp.retention_days, crate::device::camera_ftp::images_dir(&c.camera_ftp, &data_dir))
            };
            match purge(&db, days).await {
                Ok(p) if p.total() > 0 => {
                    event_log::log(&db, Level::Info, "server", "retention", format!("清除 {days} 天前的資料：包裹 {}、列印任務 {}、回報 {}、系統事件 {}", p.parcels, p.print_jobs, p.reports, p.events));
                }
                Ok(_) => {}
                Err(e) => event_log::log(&db, Level::Error, "server", "retention", format!("清除舊資料失敗: {e}")),
            }
            match crate::device::camera_ftp::purge(&db, &images_dir, image_days).await {
                Ok(n) if n > 0 => event_log::log(&db, Level::Info, "server", "retention", format!("清除 {image_days} 天前的讀碼站照片 {n} 張")),
                Ok(_) => {}
                Err(e) => event_log::log(&db, Level::Error, "server", "retention", format!("清除舊照片失敗: {e}")),
            }
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(3600)) => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_db() -> DbPool {
        let dir = std::env::temp_dir().join(format!("ret-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        crate::db::init(&dir).await.unwrap()
    }

    #[tokio::test]
    async fn 只清超過天數的資料_未送出的回報保留() {
        let db = test_db().await;
        let now = crate::db::now_ms();
        let old_ms = now - 20 * 86_400_000;
        let old_ts = crate::db::local_ts(old_ms);
        let new_ts = crate::db::local_ts(now);
        for (ulid, ts, ms) in [("OLD", &old_ts, old_ms), ("NEW", &new_ts, now)] {
            sqlx::query("INSERT INTO parcels (ulid, barcode, status, started_at, started_ms, updated_ms) VALUES (?, 'x', 3, ?, ?, ?)")
                .bind(ulid).bind(ts).bind(ms).bind(ms).execute(&db).await.unwrap();
            sqlx::query("INSERT INTO parcel_events (parcel_id, ts_ms, source, kind) VALUES ((SELECT id FROM parcels WHERE ulid = ?), ?, 'belt', 'P')")
                .bind(ulid).bind(ms).execute(&db).await.unwrap();
        }
        let spool = std::env::temp_dir().join(format!("spool-{}.tspl", ulid::Ulid::generate()));
        std::fs::write(&spool, b"x").unwrap();
        sqlx::query("INSERT INTO print_jobs (barcode, chute_code, printer_port, tspl_path, status, created_ms) VALUES ('x', 'L1', '1-8.1', ?, 'failed', ?)")
            .bind(spool.to_string_lossy().into_owned()).bind(old_ms).execute(&db).await.unwrap();
        sqlx::query("INSERT INTO print_jobs (barcode, chute_code, printer_port, tspl_path, status, created_ms) VALUES ('y', 'L1', '1-8.1', '/nonexistent', 'pending', ?)")
            .bind(old_ms).execute(&db).await.unwrap();
        for (status, ms) in [("success", old_ms), ("pending", old_ms), ("success", now)] {
            sqlx::query("INSERT INTO report_queue (response_id, payload, status, created_ms) VALUES (1, '{}', ?, ?)").bind(status).bind(ms).execute(&db).await.unwrap();
        }
        sqlx::query("INSERT INTO event_log (level, category, action, message, created_at) VALUES ('info', 'x', 'y', 'z', ?)").bind(&old_ts).execute(&db).await.unwrap();

        let p = purge(&db, 15).await.unwrap();
        assert_eq!(p, Purged { parcels: 1, print_jobs: 1, reports: 1, events: 1 });
        assert!(!spool.exists(), "點陣檔要一起清");
        let left: Vec<(String,)> = sqlx::query_as("SELECT ulid FROM parcels").fetch_all(&db).await.unwrap();
        assert_eq!(left, vec![("NEW".to_string(),)]);
        let ev: i64 = sqlx::query_scalar("SELECT count(*) FROM parcel_events").fetch_one(&db).await.unwrap();
        assert_eq!(ev, 1, "事件隨包裹級聯刪除");
        let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM report_queue WHERE status = 'pending'").fetch_one(&db).await.unwrap();
        assert_eq!(pending, 1);
        let waiting: i64 = sqlx::query_scalar("SELECT count(*) FROM print_jobs WHERE status = 'pending'").fetch_one(&db).await.unwrap();
        assert_eq!(waiting, 1, "還沒印出去的任務不清");

        assert_eq!(purge(&db, 0).await.unwrap().total(), 0, "0 天 = 不清");
    }
}
