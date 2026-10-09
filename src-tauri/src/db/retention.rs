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
    let Some(cutoff_day) = (days > 0).then(|| crate::db::retention_cutoff_day(days)).flatten() else {
        return Ok(Purged::default());
    };
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
    let print_jobs = crate::db::delete_in_batches(db, "print_jobs", "created_ms < ? AND status IN ('done', 'failed')", cutoff_ms).await?;
    // 還沒送出去的回報不刪，重啟後仍要續送
    let reports = crate::db::delete_in_batches(db, "report_queue", "created_ms < ? AND status IN ('success', 'failed')", cutoff_ms).await?;
    // parcel_events 隨 parcels 級聯刪除
    // 一件包裹連帶十幾筆事件（外鍵級聯），是最重的一張；分批刪才不會長時間擋住狀態機寫入
    let parcels = crate::db::delete_in_batches(db, "parcels", "started_at < ?", cutoff_ts.clone()).await?;
    let events = crate::db::delete_in_batches(db, "event_log", "created_at < ?", cutoff_ts.clone()).await?;
    // 卡件事件跟包裹同一個保留期
    crate::db::delete_in_batches(db, "jam_events", "created_at < ?", cutoff_ts.clone()).await?;
    Ok(Purged { parcels, print_jobs, reports, events })
}

/// 啟動清一次，之後每小時；天數跟著設定即時變。照片另有自己的保留天數（`camera_ftp.retention_days`）
pub fn start(db: DbPool, data_dir: std::path::PathBuf, cfg: watch::Receiver<AppConfig>, cancel: CancellationToken) {
    crate::supervise::spawn("資料清理", async move {
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

    #[tokio::test]
    async fn 參照包裹的子表欄位都要有索引_否則清理時每刪一件就整表掃一次() {
        let db = test_db().await;
        let tables: Vec<(String,)> = sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_sqlx%'").fetch_all(&db).await.unwrap();
        let mut missing = Vec::new();
        for (t,) in &tables {
            let fks: Vec<(String, String)> = sqlx::query_as("SELECT \"table\", \"from\" FROM pragma_foreign_key_list(?)").bind(t).fetch_all(&db).await.unwrap();
            for (parent, col) in fks.into_iter().filter(|(parent, _)| parent == "parcels") {
                let leading: Vec<(String,)> = sqlx::query_as("SELECT ii.name FROM pragma_index_list(?) il JOIN pragma_index_info(il.name) ii WHERE ii.seqno = 0")
                .bind(t)
                .fetch_all(&db)
                .await
                .unwrap();
                if !leading.iter().any(|(c,)| *c == col) {
                    missing.push(format!("{t}.{col} → {parent}"));
                }
            }
        }
        assert!(missing.is_empty(), "沒有索引的外鍵欄位：{missing:?}");
    }


    #[tokio::test]
    async fn 保留天數大到超出日期範圍_不可崩潰_視為沒有那麼舊的資料() {
        let db = test_db().await;
        let p = purge(&db, u32::MAX).await.unwrap();
        assert_eq!(p.total(), 0);
    }


    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn 大量過期包裹一次清理時_分揀寫入不可被擋住太久() {
        // 保留天數改短、或從不清理改成要清時，一次要刪的可能是幾十天的資料；
        // 一道 DELETE 刪完會一直占著唯一的寫入鎖，狀態機的包裹寫入只能等（最多 5 秒就放棄）
        let db = test_db().await;
        let old_ms = crate::db::now_ms() - 40 * 86_400_000;
        let old_ts = crate::db::local_ts(old_ms);
        sqlx::query(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 20000)
             INSERT INTO parcels (ulid, barcode, status, started_at, started_ms, updated_ms) SELECT 'OLD' || i, 'x', 3, ?1, ?2, ?2 FROM n",
        )
        .bind(&old_ts)
        .bind(old_ms)
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "WITH RECURSIVE k(j) AS (SELECT 1 UNION ALL SELECT j + 1 FROM k WHERE j < 10)
             INSERT INTO parcel_events (parcel_id, ts_ms, source, kind) SELECT p.id, ?1, 'belt', 'P' FROM parcels p CROSS JOIN k",
        )
        .bind(old_ms)
        .execute(&db)
        .await
        .unwrap();

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer = {
            let (db, stop) = (db.clone(), stop.clone());
            tokio::spawn(async move {
                let mut worst = Duration::ZERO;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let t = std::time::Instant::now();
                    sqlx::query("INSERT INTO event_log (level, category, action, message, created_at) VALUES ('info', 't', 't', 't', datetime('now','localtime'))")
                        .execute(&db)
                        .await
                        .unwrap();
                    worst = worst.max(t.elapsed());
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                worst
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        let p = purge(&db, 15).await.unwrap();
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let worst = writer.await.unwrap();

        assert_eq!(p.parcels, 20_000, "過期包裹要全部刪掉");
        let ev: i64 = sqlx::query_scalar("SELECT count(*) FROM parcel_events").fetch_one(&db).await.unwrap();
        assert_eq!(ev, 0, "事件跟著包裹刪掉");
        eprintln!("清理期間寫入最長等了 {worst:?}");
        assert!(worst < Duration::from_millis(300), "清理期間寫入最長等了 {worst:?}");
    }

}
