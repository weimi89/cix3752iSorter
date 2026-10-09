//! 啟動流程：設定 → DB → 裝置層 → 狀態機 → 中介機 → 列印。
//! 桌面模式（Tauri 視窗）與 `--headless`（開發／模擬用，不開視窗）共用這一份，
//! 兩者只差在誰擁有網頁伺服器的生命週期。

use std::path::{Path, PathBuf};
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::{AppState, chute, config, db, device, event_log, label, middleware, runtime, tracker};

/// 啟動完成後交給呼叫端的東西：共享狀態與網頁伺服器要綁的位址
pub struct Started {
    pub app: AppState,
    pub bind: String,
}

pub async fn bootstrap(config_path: &Path, data_dir: &Path, cancel: CancellationToken) -> anyhow::Result<Started> {
    let cfg = config::AppConfig::load_or_create(config_path).await?;
    // 日誌與裝置訊號留檔都放 data/logs，保留天數跟包裹資料一致
    let logs_dir = data_dir.join("logs");
    let config = config::ConfigHandle::new(config_path.to_path_buf(), cfg.clone());
    crate::log::attach_file(&logs_dir);
    device::signal_log::init(logs_dir, config.subscribe());
    let db = db::init(data_dir).await?;
    crate::supervise::log_panics_to(db.clone());
    let read_db = db::init_read(data_dir, &db).await?;
    // 統計用的一次性回填，失敗只記錄，不能擋分揀開機
    if let Err(e) = db::abnormal_kind::backfill_daily_stats(&db, &cfg.general.default_chute).await {
        tracing::error!("daily_stats 異常三分類回填失敗（下次啟動再試）: {e}");
    }
    db::retention::start(db.clone(), data_dir.to_path_buf(), config.subscribe(), cancel.clone());

    let app = AppState {
        config: config.clone(),
        db: db.clone(),
        read_db,
        data_dir: data_dir.to_path_buf(),
        started_at: Instant::now(),
        runtime: runtime::Runtime::default(),
        tracker: std::sync::Arc::new(std::sync::OnceLock::new()),
        resolver: std::sync::Arc::new(std::sync::OnceLock::new()),
        printer: std::sync::Arc::new(std::sync::OnceLock::new()),
        tasks: tokio_util::task::TaskTracker::new(),
    };
    event_log::log(&db, event_log::Level::Info, "server", "start", format!("版本 {} 啟動", env!("CARGO_PKG_VERSION")));

    // 裝置層：各裝置一個 task，事件匯進同一條 channel 給狀態機
    let (dev_tx, dev_rx) = tokio::sync::mpsc::channel(4096);
    let belt = device::belt::spawn(config.subscribe(), dev_tx.clone(), cancel.clone(), &app.tasks);
    let sorter = device::sorter::spawn(config.subscribe(), dev_tx.clone(), cancel.clone());
    device::camera::spawn(config.subscribe(), dev_tx.clone(), cancel.clone());
    // 讀碼站照片：讀碼器用 FTP 上傳每件的圖，本程式收下當收件證據
    device::camera_ftp::spawn(db.clone(), data_dir.to_path_buf(), config.subscribe(), cancel.clone());
    drop(dev_tx);
    // 面單通道：狀態機接受格口決定後才把面單丟進來，列印端另一頭收
    let (label_tx, label_rx) = tokio::sync::mpsc::channel::<label::LabelJob>(256);
    let (handle, ports) = tracker::spawn(app.clone(), tracker::Devices { belt, sorter }, dev_rx, label_tx, cancel.clone()).await?;
    let _ = app.tracker.set(handle.clone());

    // 中介機：格口解析、回報佇列、設備異常廣播
    let mw = middleware::Middleware::new(config.subscribe());
    middleware::report_queue::ReportQueue::new(db.clone()).start_worker(mw.clone(), cancel.clone());
    let chutes = tracker::load_chutes(&db).await?;
    let resolver = chute::ChuteResolver::spawn(app.clone(), mw.clone(), handle.clone(), chutes, ports.chute_rx, cancel.clone());
    let _ = app.resolver.set(resolver.clone());
    {
        let db = db.clone();
        let mw = mw.clone();
        let mut jam_rx = ports.jam_rx;
        crate::supervise::spawn("卡件告警", async move {
            while let Some(alert) = jam_rx.recv().await {
                match alert {
                    tracker::JamAlert::Chute(chute_no) => {
                        event_log::log(&db, event_log::Level::Warn, "sorter", "jam", format!("M{chute_no} 卡件"));
                        mw.device_alert("PARCEL_JAM", &format!("M{chute_no} 卡件")).await;
                    }
                    tracker::JamAlert::Hotspot { module, count } => {
                        let msg = format!("模組 M{module} 一小時內第 {count} 次卡件，請檢查該段分流機構");
                        event_log::log(&db, event_log::Level::Warn, "sorter", "jam_hotspot", msg.clone());
                        crate::event_bus::emit("parcel-alert", serde_json::json!({ "kind": "jam_hotspot", "module": module, "count": count, "message": msg }));
                        mw.device_alert("JAM_HOTSPOT", &msg).await;
                    }
                }
            }
        });
    }

    // 列印：已下載的面單 → 點陣 → 每台印表機各自的佇列 worker
    let printer = label::PrintService::new(db.clone(), config.subscribe(), mw.clone(), data_dir, cancel.clone(), app.tasks.clone());
    let _ = app.printer.set(printer.clone());
    label::spawn_pipeline(app.clone(), printer, label_rx, cancel.clone());

    Ok(Started { app, bind: cfg.server.bind.clone() })
}

/// 等到收到結束訊號（Ctrl-C 或 SIGTERM）為止。
///
/// 關機、systemd／supervisor 停服務、`sorter-switch to-old` 送的都是 SIGTERM；沒有登記處理的話系統直接結束程序，
/// 結束前等寫入佇列寫完那一步（`supervise::drain`）根本跑不到。桌面視窗框架本身不處理這個訊號，要自己接
pub async fn wait_terminate() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(e) => {
                tracing::error!("登記 SIGTERM 處理失敗，只能接 Ctrl-C: {e}");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// 桌面模式的預設路徑：跟 LabelPrint 一樣放在使用者的應用資料夾，
/// 不依賴工作目錄（從應用選單啟動時工作目錄是家目錄）
pub fn default_paths(app_data_dir: &Path) -> (PathBuf, PathBuf) {
    (app_data_dir.join("config.toml"), app_data_dir.join("data"))
}

#[cfg(all(test, unix))]
mod tests {
    #[tokio::test]
    async fn 收到_sigterm_要走收尾流程_不可被系統直接結束() {
        // 桌面版沒登記處理的話，SIGTERM 照系統預設直接結束程序：關機、切換腳本 to-old、systemd 停服務
        // 都是送這個訊號，寫入佇列裡還沒寫的包裹就沒了。這個測試對自己送 SIGTERM——沒處理的話整個測試程序會被結束
        let got = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let g = got.clone();
        tokio::spawn(async move {
            super::wait_terminate().await;
            g.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let st = std::process::Command::new("kill").arg("-TERM").arg(std::process::id().to_string()).status().unwrap();
        assert!(st.success());
        for _ in 0..40 {
            if got.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("送了 SIGTERM 兩秒內沒有收到通知");
    }
}
