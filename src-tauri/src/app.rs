//! 啟動流程：設定 → DB → 裝置層 → 狀態機 → 中介機 → 列印 → 更新檢查。
//! 桌面模式（Tauri 視窗）與 `--headless`（supervisor／systemd）共用這一份，
//! 兩者只差在誰擁有網頁伺服器的生命週期。

use std::path::{Path, PathBuf};
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::{AppState, chute, config, db, device, event_log, label, middleware, runtime, tracker, updater};

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
    db::retention::start(db.clone(), config.subscribe(), cancel.clone());

    let app = AppState {
        config: config.clone(),
        db: db.clone(),
        data_dir: data_dir.to_path_buf(),
        started_at: Instant::now(),
        runtime: runtime::Runtime::default(),
        tracker: std::sync::Arc::new(std::sync::OnceLock::new()),
        resolver: std::sync::Arc::new(std::sync::OnceLock::new()),
        printer: std::sync::Arc::new(std::sync::OnceLock::new()),
        updater: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    event_log::log(&db, event_log::Level::Info, "server", "start", format!("版本 {} 啟動", env!("CARGO_PKG_VERSION")));

    // 裝置層：各裝置一個 task，事件匯進同一條 channel 給狀態機
    let (dev_tx, dev_rx) = tokio::sync::mpsc::channel(4096);
    let belt = device::belt::spawn(config.subscribe(), dev_tx.clone(), cancel.clone());
    let sorter = device::sorter::spawn(config.subscribe(), dev_tx.clone(), cancel.clone());
    device::camera::spawn(config.subscribe(), dev_tx.clone(), cancel.clone());
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
        tokio::spawn(async move {
            while let Some(chute_no) = jam_rx.recv().await {
                event_log::log(&db, event_log::Level::Warn, "sorter", "jam", format!("M{chute_no} 卡件"));
                mw.device_alert("PARCEL_JAM", &format!("M{chute_no} 卡件")).await;
            }
        });
    }

    // 列印：已下載的面單 → 點陣 → 每台印表機各自的佇列 worker
    let printer = label::PrintService::new(db.clone(), config.subscribe(), mw.clone(), data_dir, cancel.clone());
    let _ = app.printer.set(printer.clone());
    label::spawn_pipeline(app.clone(), printer, label_rx, cancel.clone());

    // 自動更新（headless 用：定期查 latest.json，由網頁觸發換檔；桌面模式由 Tauri updater 接手）
    let up = updater::Updater::new(db.clone(), config.subscribe(), data_dir);
    up.start_background(cancel.clone());
    let _ = app.updater.set(up);

    Ok(Started { app, bind: cfg.server.bind.clone() })
}

/// 桌面模式的預設路徑：跟 LabelPrint 一樣放在使用者的應用資料夾，
/// 不依賴工作目錄（從應用選單啟動時工作目錄是家目錄）
pub fn default_paths(app_data_dir: &Path) -> (PathBuf, PathBuf) {
    (app_data_dir.join("config.toml"), app_data_dir.join("data"))
}
