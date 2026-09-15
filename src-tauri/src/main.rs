#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use clap::Parser;
use tokio_util::sync::CancellationToken;

use cix3752i_sorter::{app, log, server};

/// 智配通 分揀控制
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// 設定檔路徑（桌面模式預設在應用資料夾；headless 預設 ./config.toml）
    #[arg(long)]
    config: Option<PathBuf>,
    /// 資料目錄（SQLite、面單點陣檔）
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// 不開視窗只跑服務：開發機接模擬器、或 CI 跑整合測試用；現場一律裝桌面版。
    /// 環境變數 `CIX_HEADLESS=1`（或 true／yes／on）同效；clap 預設只認 true／false，`=1` 會被當非法值拒絕
    #[arg(long, env = "CIX_HEADLESS", value_parser = clap::builder::FalseyValueParser::new())]
    headless: bool,
}

fn main() -> anyhow::Result<()> {
    log::init();
    let cli = Cli::parse();
    if cli.headless {
        let r = tokio::runtime::Runtime::new()?.block_on(run_headless(cli));
        log::flush();
        return r;
    }
    cix3752i_sorter::desktop::run(cli.config, cli.data_dir);
    log::flush();
    Ok(())
}

async fn run_headless(cli: Cli) -> anyhow::Result<()> {
    let config_path = cli.config.unwrap_or_else(|| PathBuf::from("config.toml"));
    let data_dir = cli.data_dir.unwrap_or_else(|| PathBuf::from("data"));
    let cancel = CancellationToken::new();
    let started = app::bootstrap(&config_path, &data_dir, cancel.clone()).await?;

    // supervisor／systemd 停服務送的是 SIGTERM，不只 Ctrl-C；兩種都要走乾淨關閉（裝置連線收線、日誌 flush）
    let shutdown = {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            #[cfg(unix)]
            {
                let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
            tracing::info!("收到結束訊號，開始關閉");
            cancel.cancel();
        })
    };
    server::serve(started.app, &started.bind, cancel.clone()).await?;
    shutdown.abort();
    tracing::info!("已關閉");
    Ok(())
}
