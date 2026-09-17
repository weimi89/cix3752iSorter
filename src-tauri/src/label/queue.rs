//! 列印佇列：任務先落 `print_jobs` 表與點陣檔，再由每台印表機各自的 worker 依序送印。
//!
//! - 印表機找不到（USB 掉線）或寫入失敗：任務留著重試，不丟；重試到設定次數就發告警並拉長間隔
//! - 寫裝置檔放 `spawn_blocking` 並設逾時，卡住的 USB 不會拖住其他印表機或主迴圈
//! - 超過一小時仍印不出來的任務標為失敗，網頁可手動重送

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{Notify, watch};
use tokio_util::sync::CancellationToken;

use super::usb;
use crate::config::AppConfig;
use crate::db::{DbPool, now_ms};
use crate::event_bus;
use crate::event_log::{self, Level};
use crate::middleware::Middleware;

/// 任務放著超過這麼久還沒印出來 → 標失敗
const GIVE_UP_AFTER_MS: i64 = 3_600_000;
/// 單次寫裝置檔的逾時
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);
/// 同一台印表機同型別告警最短間隔
const ALERT_THROTTLE_MS: i64 = 60_000;

#[derive(Clone)]
pub struct PrintService {
    db: DbPool,
    cfg: watch::Receiver<AppConfig>,
    mw: Middleware,
    spool_dir: PathBuf,
    cancel: CancellationToken,
    workers: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
}

pub struct NewJob<'a> {
    pub parcel_ulid: Option<&'a str>,
    pub barcode: &'a str,
    pub chute_code: &'a str,
    pub printer_port: &'a str,
    pub profile: Option<&'a str>,
    pub tspl: &'a [u8],
    /// 送印延遲（毫秒）
    pub delay_ms: i64,
}

/// 找裝置檔。測試／開發機可設 `CIX_PRINT_FAKE_DIR`：該目錄下存在名為埠位的檔案就當作印表機。
pub fn resolve_device(port: &str) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CIX_PRINT_FAKE_DIR") {
        let p = Path::new(&dir).join(port);
        return p.exists().then_some(p);
    }
    usb::find_printer(port)
}

impl PrintService {
    pub fn new(db: DbPool, cfg: watch::Receiver<AppConfig>, mw: Middleware, data_dir: &Path, cancel: CancellationToken) -> Self {
        let spool_dir = data_dir.join("print");
        let _ = std::fs::create_dir_all(&spool_dir);
        let me = Self { db, cfg, mw, spool_dir, cancel, workers: Arc::new(Mutex::new(HashMap::new())) };
        me.resume();
        me
    }

    /// 啟動時：上次卡在 printing 的任務改回 pending，並為每個有待印任務的埠位起 worker
    fn resume(&self) {
        let me = self.clone();
        tokio::spawn(async move {
            let _ = sqlx::query("UPDATE print_jobs SET status = 'pending' WHERE status = 'printing'").execute(&me.db).await;
            let ports: Vec<(String,)> = sqlx::query_as("SELECT DISTINCT printer_port FROM print_jobs WHERE status = 'pending'")
                .fetch_all(&me.db)
                .await
                .unwrap_or_default();
            for (port,) in ports {
                me.wake(&port);
            }
        });
    }

    /// 入列並喚醒該埠位的 worker
    pub async fn enqueue(&self, job: NewJob<'_>) -> anyhow::Result<i64> {
        let now = now_ms();
        let name = format!("{}-{}.tspl", now, ulid::Ulid::generate());
        let path = self.spool_dir.join(&name);
        crate::fs_atomic::write_async(&path, job.tspl).await?;
        let row: (i64,) = sqlx::query_as(
            "INSERT INTO print_jobs (parcel_id, barcode, chute_code, printer_port, profile, tspl_path, status, not_before_ms, created_ms)
             VALUES ((SELECT id FROM parcels WHERE ulid = ?1), ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8) RETURNING id",
        )
        .bind(job.parcel_ulid)
        .bind(job.barcode)
        .bind(job.chute_code)
        .bind(job.printer_port)
        .bind(job.profile)
        .bind(path.to_string_lossy().into_owned())
        .bind(now + job.delay_ms)
        .bind(now)
        .fetch_one(&self.db)
        .await?;
        event_bus::emit("print-job", serde_json::json!({ "id": row.0, "status": "pending", "chute": job.chute_code, "barcode": job.barcode }));
        self.wake(job.printer_port);
        Ok(row.0)
    }

    /// 網頁手動重送
    pub async fn retry(&self, id: i64) -> anyhow::Result<()> {
        let port: Option<(String,)> = sqlx::query_as("UPDATE print_jobs SET status = 'pending', attempts = 0, last_error = NULL, not_before_ms = 0 WHERE id = ? RETURNING printer_port")
            .bind(id)
            .fetch_optional(&self.db)
            .await?;
        if let Some((port,)) = port {
            self.wake(&port);
        }
        Ok(())
    }

    /// 直接送一段 TSPL（測試頁），不進佇列
    pub async fn print_raw(&self, port: &str, data: Vec<u8>) -> anyhow::Result<()> {
        let dev = resolve_device(port).ok_or_else(|| anyhow::anyhow!("找不到埠位 {port} 的印表機"))?;
        let r = tokio::time::timeout(WRITE_TIMEOUT, tokio::task::spawn_blocking(move || usb::write_device(&dev, &data))).await;
        match r {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(e))) => Err(anyhow::anyhow!("寫入失敗: {e}")),
            Ok(Err(e)) => Err(anyhow::anyhow!("寫入工作中斷: {e}")),
            Err(_) => Err(anyhow::anyhow!("寫入逾時（{}s）", WRITE_TIMEOUT.as_secs())),
        }
    }

    fn wake(&self, port: &str) {
        let notify = {
            let mut w = self.workers.lock().unwrap();
            match w.get(port) {
                Some(n) => n.clone(),
                None => {
                    let n = Arc::new(Notify::new());
                    w.insert(port.to_string(), n.clone());
                    tokio::spawn(self.clone().worker(port.to_string(), n.clone()));
                    n
                }
            }
        };
        notify.notify_one();
    }

    async fn worker(self, port: String, notify: Arc<Notify>) {
        tracing::info!(%port, "印表機 worker 啟動");
        let mut attempts: u32 = 0;
        let mut last_alert: HashMap<&'static str, i64> = HashMap::new();
        loop {
            if self.cancel.is_cancelled() {
                break;
            }
            let now = now_ms();
            let job: Option<(i64, String, String, String, Option<String>, i64)> = sqlx::query_as(
                "UPDATE print_jobs SET status = 'printing'
                 WHERE id = (SELECT id FROM print_jobs WHERE printer_port = ?1 AND status = 'pending' AND not_before_ms <= ?2 ORDER BY id LIMIT 1)
                 RETURNING id, barcode, chute_code, tspl_path, profile, created_ms",
            )
            .bind(&port)
            .bind(now)
            .fetch_optional(&self.db)
            .await
            .unwrap_or(None);

            let Some((id, barcode, chute, tspl_path, profile, created_ms)) = job else {
                tokio::select! {
                    _ = self.cancel.cancelled() => break,
                    _ = notify.notified() => {}
                    _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                }
                continue;
            };

            let (retry, retry_interval) = {
                let cfg = self.cfg.borrow();
                let p = profile.as_deref().and_then(|n| cfg.print.profiles.get(n)).cloned().unwrap_or_default();
                (p.retry.max(1), Duration::from_millis(p.retry_interval_ms.max(50)))
            };

            if now - created_ms > GIVE_UP_AFTER_MS {
                self.mark(id, "failed", Some("超過一小時未能列印")).await;
                event_log::log(&self.db, Level::Error, "printer", "give_up", format!("{chute} 條碼 {barcode} 超過一小時未能列印，標為失敗"));
                continue;
            }

            let Some(dev) = resolve_device(&port) else {
                attempts += 1;
                self.mark(id, "pending", Some("找不到印表機")).await;
                if attempts >= retry {
                    attempts = 0;
                    self.alert(&mut last_alert, "USB_DISCONNECT", &format!("{chute} 斷線"), &port).await;
                    tokio::time::sleep(Duration::from_secs(3)).await;
                } else {
                    tokio::time::sleep(retry_interval).await;
                }
                continue;
            };

            let data = match tokio::fs::read(&tspl_path).await {
                Ok(d) => d,
                Err(e) => {
                    self.mark(id, "failed", Some(&format!("點陣檔讀取失敗: {e}"))).await;
                    event_log::log(&self.db, Level::Error, "printer", "spool_missing", format!("{chute} 條碼 {barcode} 點陣檔遺失: {e}"));
                    continue;
                }
            };
            let dev2 = dev.clone();
            let r = tokio::time::timeout(WRITE_TIMEOUT, tokio::task::spawn_blocking(move || usb::write_device(&dev2, &data))).await;
            let err = match r {
                Ok(Ok(Ok(()))) => None,
                Ok(Ok(Err(e))) => Some(format!("寫入失敗: {e}")),
                Ok(Err(e)) => Some(format!("寫入工作中斷: {e}")),
                Err(_) => Some(format!("寫入逾時（{}s）", WRITE_TIMEOUT.as_secs())),
            };
            match err {
                None => {
                    attempts = 0;
                    self.mark(id, "done", None).await;
                    let _ = tokio::fs::remove_file(&tspl_path).await;
                    tracing::info!(%port, dev = %dev.display(), %chute, %barcode, "面單已送印");
                    event_bus::emit("print-job", serde_json::json!({ "id": id, "status": "done", "chute": chute, "barcode": barcode }));
                }
                Some(e) => {
                    attempts += 1;
                    self.mark(id, "pending", Some(&e)).await;
                    tracing::warn!(%port, attempt = attempts, "{e}");
                    if attempts >= retry {
                        attempts = 0;
                        self.alert(&mut last_alert, "PRINTER_ERROR", &format!("{chute} 印表機寫入失敗"), &port).await;
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    } else {
                        tokio::time::sleep(retry_interval).await;
                    }
                }
            }
        }
    }

    async fn mark(&self, id: i64, status: &str, error: Option<&str>) {
        let finished = if status == "done" || status == "failed" { Some(now_ms()) } else { None };
        let _ = sqlx::query("UPDATE print_jobs SET status = ?, attempts = attempts + ?, last_error = ?, finished_ms = COALESCE(?, finished_ms) WHERE id = ?")
            .bind(status)
            .bind(if status == "done" { 1 } else if error.is_some() { 1 } else { 0 })
            .bind(error)
            .bind(finished)
            .bind(id)
            .execute(&self.db)
            .await;
    }

    async fn alert(&self, last: &mut HashMap<&'static str, i64>, kind: &'static str, message: &str, port: &str) {
        let now = now_ms();
        if now - last.get(kind).copied().unwrap_or(0) < ALERT_THROTTLE_MS {
            return;
        }
        last.insert(kind, now);
        event_log::log(&self.db, Level::Error, "printer", kind, format!("{message}（埠位 {port}）"));
        event_bus::emit("printer-alert", serde_json::json!({ "type": kind, "message": message, "port": port }));
        self.mw.device_alert(kind, message).await;
    }
}
