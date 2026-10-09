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
    install_panic_hook();
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

/// 崩潰時把原因與位置寫進日誌。預設只印到標準錯誤，桌面版從選單啟動時那裡沒人接，
/// 崩潰就完全不留紀錄。原本的處理照樣接著跑（開發時終端機仍看得到）；重複呼叫只裝一次
pub fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let reason = panic_text(info.payload());
            let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
            let thread = std::thread::current().name().unwrap_or("未命名").to_string();
            tracing::error!(%thread, %location, "程式崩潰：{reason}");
            prev(info);
        }));
    });
}

/// 崩潰內容轉成文字（`panic!` 帶的是 `&str` 或 `String`）
pub fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "（無法辨識的崩潰內容）".to_string())
}

/// 程式結束前呼叫：把背景寫檔執行緒的緩衝全部落檔，收尾訊息與 panic 前最後幾行才不會丟
pub fn flush() {
    if let Ok(mut g) = FILE_GUARD.lock() {
        drop(g.take());
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    /// 收集 tracing 事件文字的測試用寫入端
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn 崩潰要寫進日誌_桌面版沒有終端機_只印標準錯誤等於沒留紀錄() {
        super::install_panic_hook();
        let cap = Capture::default();
        let c2 = cap.clone();
        let sub = tracing_subscriber::fmt().with_writer(move || c2.clone()).with_ansi(false).finish();
        tracing::subscriber::with_default(sub, || {
            let _ = std::panic::catch_unwind(|| panic!("日誌崩潰測試 {}", 42));
        });
        let text = String::from_utf8(cap.0.lock().unwrap().clone()).unwrap();
        assert!(text.contains("日誌崩潰測試 42"), "日誌要有崩潰原因，實際：{text:?}");
        assert!(text.contains("log.rs"), "日誌要有崩潰位置，實際：{text:?}");
    }
}
