//! 列印佇列：任務先落 `print_jobs` 表與點陣檔，再由每台印表機各自的 worker 依序送印。
//!
//! - 印表機找不到（USB 掉線）或寫入失敗：任務留著重試，不丟；重試到設定次數就發告警並拉長間隔
//! - 寫裝置檔放 `spawn_blocking` 並設逾時，卡住的 USB 不會拖住其他印表機或主迴圈
//! - 同一台印表機同一時間只能有一個寫入（佇列與測試頁共用一把鎖，寫入真正結束才放）：
//!   卡住的寫入無法取消，再開一個的話印表機恢復時兩份都會印出、內容還可能夾在一起
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
    tasks: tokio_util::task::TaskTracker,
    workers: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
    /// 每台印表機的寫入鎖（見模組說明）
    writing: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    write_timeout: Duration,
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
    pub fn new(db: DbPool, cfg: watch::Receiver<AppConfig>, mw: Middleware, data_dir: &Path, cancel: CancellationToken, tasks: tokio_util::task::TaskTracker) -> Self {
        let spool_dir = data_dir.join("print");
        let _ = std::fs::create_dir_all(&spool_dir);
        let me = Self { db, cfg, mw, spool_dir, cancel, tasks, workers: Arc::new(Mutex::new(HashMap::new())), writing: Arc::new(Mutex::new(HashMap::new())), write_timeout: WRITE_TIMEOUT };
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
        crate::fs_atomic::write_async(&path, job.tspl, crate::fs_atomic::Durability::Synced).await?;
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

    /// 直接送一段 TSPL（測試頁），不進佇列。這台正在寫（列印中或卡住）就不送，不排隊
    pub async fn print_raw(&self, port: &str, data: Vec<u8>) -> anyhow::Result<()> {
        let dev = resolve_device(port).ok_or_else(|| anyhow::anyhow!("找不到埠位 {port} 的印表機"))?;
        let guard = self.port_lock(port).try_lock_owned().map_err(|_| anyhow::anyhow!("這台印表機正在列印或沒有回應，請稍後再試"))?;
        let write = tokio::task::spawn_blocking(move || {
            let _writing = guard;
            usb::write_device(&dev, &data)
        });
        match tokio::time::timeout(self.write_timeout, write).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(e))) => Err(anyhow::anyhow!("寫入失敗: {e}")),
            Ok(Err(e)) => Err(anyhow::anyhow!("寫入工作中斷: {e}")),
            Err(_) => Err(anyhow::anyhow!("印表機沒有回應（{}s），恢復後會印出這張測試頁", self.write_timeout.as_secs())),
        }
    }

    fn port_lock(&self, port: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.writing.lock().unwrap().entry(port.to_string()).or_default().clone()
    }

    fn wake(&self, port: &str) {
        let notify = {
            let mut w = self.workers.lock().unwrap();
            match w.get(port) {
                Some(n) => n.clone(),
                None => {
                    let n = Arc::new(Notify::new());
                    w.insert(port.to_string(), n.clone());
                    crate::supervise::spawn_tracked(format!("列印佇列 {port}"), &self.tasks, self.clone().worker(port.to_string(), n.clone()));
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
                    self.pause(Duration::from_secs(3)).await;
                } else {
                    self.pause(retry_interval).await;
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
            // 測試頁還卡著時在這裡等它寫完；鎖跟著寫入走，寫入真正結束才放
            let guard = tokio::select! {
                _ = self.cancel.cancelled() => break,
                g = self.port_lock(&port).lock_owned() => g,
            };
            let dev2 = dev.clone();
            let mut write = tokio::task::spawn_blocking(move || {
                let _writing = guard;
                usb::write_device(&dev2, &data)
            });
            let r = match tokio::time::timeout(self.write_timeout, &mut write).await {
                Ok(r) => r,
                Err(_) => {
                    // 卡住的寫入無法取消：這時再開第二次寫入，印表機恢復時兩次都會印出來（同一張面單多張）。
                    // 先告警，等這一次真正結束再決定成功或重試。關閉中也照樣等（結束流程最多等 DRAIN_LIMIT）：
                    // 不等的話收尾期間印出來的面單還標著列印中，重開時會再印一次
                    self.mark(id, "printing", Some(&format!("寫入逾時（{}s），等待印表機", self.write_timeout.as_secs()))).await;
                    self.alert(&mut last_alert, "PRINTER_ERROR", &format!("{chute} 印表機無回應"), &port).await;
                    write.await
                }
            };
            let err = match r {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(format!("寫入失敗: {e}")),
                Err(e) => Some(format!("寫入工作中斷: {e}")),
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
                        self.pause(Duration::from_secs(5)).await;
                    } else {
                        self.pause(retry_interval).await;
                    }
                }
            }
        }
    }

    /// 重試前的等待；收到結束通知就提早結束（迴圈開頭會看到通知而收工）
    async fn pause(&self, d: Duration) {
        tokio::select! {
            _ = self.cancel.cancelled() => {}
            _ = tokio::time::sleep(d) => {}
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// 假印表機目錄：環境變數是整個測試程式共用的，只設一次，各測試用不同埠位
    fn fake_dir() -> &'static Path {
        static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        DIR.get_or_init(|| {
            let d = std::env::temp_dir().join(format!("printq-fake-{}", ulid::Ulid::generate()));
            std::fs::create_dir_all(&d).unwrap();
            // SAFETY: 只在這裡設一次，其他測試只讀
            unsafe { std::env::set_var("CIX_PRINT_FAKE_DIR", &d) };
            d
        })
    }

    /// 具名管道沒有讀者時，開啟寫入會一直卡住：模擬印表機無回應
    fn stuck_printer(port: &str) -> PathBuf {
        let fifo = fake_dir().join(port);
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        fifo
    }

    /// 印表機恢復：開讀取端，讀到所有寫入端都關閉為止
    async fn recover(fifo: PathBuf) -> String {
        let got = tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let mut f = std::fs::File::open(&fifo).unwrap();
            let mut buf = Vec::new();
            let deadline = std::time::Instant::now() + Duration::from_millis(1_500);
            let mut chunk = [0u8; 256];
            while std::time::Instant::now() < deadline {
                match f.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    Err(_) => break,
                }
            }
            buf
        })
        .await
        .unwrap();
        String::from_utf8_lossy(&got).into_owned()
    }

    async fn service(write_timeout: Duration) -> (PrintService, DbPool, CancellationToken, tokio_util::task::TaskTracker) {
        let dir = std::env::temp_dir().join(format!("printq-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::init(&dir).await.unwrap();
        let mut cfg = AppConfig::default();
        cfg.middleware.base_url = "http://127.0.0.1:1/".into(); // 告警不要真的送出去
        let (_tx, rx) = watch::channel(cfg);
        let cancel = CancellationToken::new();
        let tasks = tokio_util::task::TaskTracker::new();
        let mut svc = PrintService::new(db.clone(), rx.clone(), Middleware::new(rx), &dir, cancel.clone(), tasks.clone());
        svc.write_timeout = write_timeout;
        (svc, db, cancel, tasks)
    }

    async fn wait_status(db: &DbPool, want: &str) -> String {
        let mut status = String::new();
        for _ in 0..40 {
            status = sqlx::query_scalar("SELECT status FROM print_jobs").fetch_one(db).await.unwrap();
            if status == want {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        status
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn 印表機卡住時不重複送印_恢復後只印一張() {
        let port = "9-9.9";
        let fifo = stuck_printer(port);
        let (svc, db, cancel, _tasks) = service(Duration::from_millis(300)).await;
        svc.enqueue(NewJob { parcel_ulid: None, barcode: "A1", chute_code: "L1", printer_port: port, profile: None, tspl: b"<JOB-1>", delay_ms: 0 }).await.unwrap();
        // 讓 worker 經歷好幾次逾時
        tokio::time::sleep(Duration::from_millis(2_000)).await;
        let got = recover(fifo).await;
        let copies = got.matches("<JOB-1>").count();
        assert_eq!(copies, 1, "印表機恢復後收到 {copies} 份同一張面單");
        // 寫入結束後 worker 才標完成，等它一下再收工
        let status = wait_status(&db, "done").await;
        cancel.cancel();
        assert_eq!(status, "done");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn 印表機卡住時連按測試頁_不疊寫入_佇列的面單也等前一次寫完() {
        let port = "9-9.8";
        let fifo = stuck_printer(port);
        let (svc, db, cancel, _tasks) = service(Duration::from_millis(300)).await;
        let first = svc.print_raw(port, b"<TEST>".to_vec()).await;
        assert!(first.is_err(), "印表機沒回應時第一次測試頁要回逾時");
        let second = svc.print_raw(port, b"<TEST>".to_vec()).await;
        assert!(second.is_err(), "前一次還卡著，第二次不能再開一個寫入");
        svc.enqueue(NewJob { parcel_ulid: None, barcode: "A1", chute_code: "L1", printer_port: port, profile: None, tspl: b"<JOB-2>", delay_ms: 0 }).await.unwrap();
        tokio::time::sleep(Duration::from_millis(1_000)).await;
        let got = recover(fifo).await;
        assert_eq!((got.matches("<TEST>").count(), got.matches("<JOB-2>").count()), (1, 0), "恢復當下只能有卡住的那一次測試頁，收到：{got:?}");
        // 測試頁寫完才輪到面單：佇列這時才開始寫，接著再讀一次
        let fifo = fake_dir().join(port);
        let got = recover(fifo).await;
        assert_eq!(got, "<JOB-2>", "面單在測試頁之後單獨寫出");
        let status = wait_status(&db, "done").await;
        cancel.cancel();
        assert_eq!(status, "done");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn 關閉時寫入還卡著_印表機在收尾期間恢復_要記成已印_重開不可再印() {
        let port = "9-9.7";
        let fifo = stuck_printer(port);
        let (svc, db, cancel, tasks) = service(Duration::from_millis(300)).await;
        svc.enqueue(NewJob { parcel_ulid: None, barcode: "A1", chute_code: "L1", printer_port: port, profile: None, tspl: b"<JOB-3>", delay_ms: 0 }).await.unwrap();
        tokio::time::sleep(Duration::from_millis(1_000)).await;
        // 關閉：結束通知送出，收尾期間印表機恢復、把這張收完
        cancel.cancel();
        let reader = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            recover(fifo).await
        });
        assert!(crate::supervise::drain(&tasks, Duration::from_secs(10)).await, "收尾要等到這次寫入有結果");
        assert_eq!(reader.await.unwrap(), "<JOB-3>");
        let status: String = sqlx::query_scalar("SELECT status FROM print_jobs").fetch_one(&db).await.unwrap();
        assert_eq!(status, "done", "已經印出的面單要記成已印，否則重開時 resume 會再印一次");
    }
}
