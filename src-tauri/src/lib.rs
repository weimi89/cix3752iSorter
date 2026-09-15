//! 智配通 分揀控制。模組切分見 `docs/plan.md`。

pub mod app;
pub mod chute;
pub mod config;
pub mod desktop;
pub mod db;
pub mod device;
pub mod event_bus;
pub mod event_log;
pub mod fs_atomic;
pub mod label;
pub mod log;
pub mod middleware;
pub mod protocol;
pub mod runtime;
pub mod server;
pub mod tracker;
pub mod updater;

use std::path::PathBuf;
use std::time::Instant;

use config::ConfigHandle;
use db::DbPool;

/// 全程式共享狀態：各 task / handler clone 一份（內部都是 Arc / 池）。
#[derive(Clone)]
pub struct AppState {
    pub config: ConfigHandle,
    pub db: DbPool,
    pub data_dir: PathBuf,
    pub started_at: Instant,
    pub runtime: runtime::Runtime,
    /// 狀態機控制入口；啟動順序上狀態機在 server 之前建立，所以這裡一定有值
    pub tracker: std::sync::Arc<std::sync::OnceLock<tracker::TrackerHandle>>,
    pub resolver: std::sync::Arc<std::sync::OnceLock<chute::ChuteResolver>>,
    pub printer: std::sync::Arc<std::sync::OnceLock<label::PrintService>>,
}
