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
    let tasks = started.app.tasks.clone();

    // supervisor／systemd 停服務送的是 SIGTERM，不只 Ctrl-C；兩種都要走乾淨關閉（裝置連線收線、日誌 flush）
    let shutdown = {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            app::wait_terminate().await;
            tracing::info!("收到結束訊號，開始關閉");
            cancel.cancel();
        })
    };
    server::serve(started.app, &started.bind, cancel.clone()).await?;
    shutdown.abort();
    // 函式一回傳 runtime 就被丟掉、所有工作中止；寫入佇列裡的包裹與剛定格口的回報要先寫完
    if !cix3752i_sorter::supervise::drain(&tasks, cix3752i_sorter::supervise::DRAIN_LIMIT).await {
        tracing::error!("結束時寫入佇列沒在時限內寫完，部分包裹狀態或回報可能遺失");
    }
    tracing::info!("已關閉");
    Ok(())
}
