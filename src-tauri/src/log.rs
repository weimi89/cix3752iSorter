//! 全域 tracing 初始化：預設 info，可由 `RUST_LOG` 覆寫。
//!
//! 一開始只有 stdout（開發時看終端機）；資料目錄確定後再 `attach_file`
//! 加上逐日輪替的檔案輸出——桌面模式從應用選單啟動時 stdout 直接被丟掉，沒有這個檔就等於沒有日誌。

use std::io::IsTerminal;
use std::path::Path;
use std::sync::OnceLock;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::layer::Layered;
use tracing_subscriber::{EnvFilter, Layer, fmt, registry::Registry, reload};

/// 全域過濾層先掛在 Registry 上，之後的層都不各自帶 filter——
/// 事後才換進來的層若自帶 `with_filter`，tracing 會因為沒登記 FilterId 直接 panic
type Base = Layered<EnvFilter, Registry>;
type FileLayer = Box<dyn Layer<Base> + Send + Sync>;

static FILE_HANDLE: OnceLock<reload::Handle<Option<FileLayer>, Base>> = OnceLock::new();
static FILE_GUARD: std::sync::Mutex<Option<tracing_appender::non_blocking::WorkerGuard>> = std::sync::Mutex::new(None);

fn filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(if cfg!(debug_assertions) {
            "cix3752i_sorter=debug,sorter=debug,axum=info,tower_http=info,sqlx=warn,info"
        } else {
            "info"
        })
    })
}

pub fn init() {
    let (file_layer, handle) = reload::Layer::new(None::<FileLayer>);
    let _ = FILE_HANDLE.set(handle);
    let stdout = fmt::layer()
        // stdout 不是終端機時（重導到檔案）色碼只會弄髒日誌
        .with_ansi(std::io::stdout().is_terminal())
        .with_target(true)
        .with_thread_ids(false)
        .compact();
    tracing_subscriber::registry().with(filter()).with(file_layer).with(stdout).init();
}

/// 加上 `dir/sorter.YYYY-MM-DD.log` 逐日輪替檔。舊檔清理交給 `device::signal_log`（同目錄、同天數、可熱套用）。
/// 重複呼叫只生效一次。
pub fn attach_file(dir: &Path) {
    if FILE_GUARD.lock().map(|g| g.is_some()).unwrap_or(true) {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::error!(dir = %dir.display(), "日誌目錄建立失敗，只留 stdout: {e}");
        return;
    }
    let builder = tracing_appender::rolling::Builder::new().rotation(tracing_appender::rolling::Rotation::DAILY).filename_prefix("sorter").filename_suffix("log");
    let appender = match builder.build(dir) {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(dir = %dir.display(), "日誌檔建立失敗，只留 stdout: {e}");
            return;
        }
    };
    let (writer, guard) = tracing_appender::non_blocking(appender);
    if let Ok(mut g) = FILE_GUARD.lock() {
        *g = Some(guard);
    }
    let layer: FileLayer = Box::new(fmt::layer().with_writer(writer).with_ansi(false).with_target(true).compact());
    if let Some(h) = FILE_HANDLE.get() {
        if let Err(e) = h.reload(Some(layer)) {
            tracing::error!("日誌檔輸出掛載失敗: {e}");
        } else {
            tracing::info!(dir = %dir.display(), "日誌檔輸出已啟用");
        }
    }
}

/// 程式結束前呼叫：把背景寫檔執行緒的緩衝全部落檔，收尾訊息與 panic 前最後幾行才不會丟
pub fn flush() {
    if let Ok(mut g) = FILE_GUARD.lock() {
        drop(g.take());
    }
}
