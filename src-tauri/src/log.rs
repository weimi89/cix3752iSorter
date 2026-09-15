//! 全域 tracing 初始化：預設 info，可由 `RUST_LOG` 覆寫。
//!
//! 輸出到 stdout，交給 supervisor / systemd-journald 做輪替；不再自己寫 `data/*.log`。

use std::io::IsTerminal;

use tracing_subscriber::EnvFilter;

pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(if cfg!(debug_assertions) {
            "cix3752i_sorter=debug,sorter=debug,axum=info,tower_http=info,sqlx=warn,info"
        } else {
            "info"
        })
    });

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        // 跑在 supervisor / systemd 底下時 stdout 不是終端機，色碼只會弄髒日誌檔
        .with_ansi(std::io::stdout().is_terminal())
        .with_target(true)
        .with_thread_ids(false)
        .compact()
        .init();
}
