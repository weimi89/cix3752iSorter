//! 桌面模式：Tauri 視窗 + 內建網頁伺服器。
//!
//! 視窗載入內嵌的前端（tauri://localhost），前端透過 `backend_base_url` 拿到本機
//! 網頁伺服器位址後，其餘資料一律走與網頁版相同的 REST／SSE —— 同一份前端在瀏覽器與桌面都能跑。

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;
use tokio_util::sync::CancellationToken;

use crate::{AppState, app, server, updater};

/// 給 Tauri command 用的共享狀態
pub struct Desktop {
    pub app: AppState,
    pub bind: String,
}

/// 前端在桌面模式要打的 API 位址；bind 若是 0.0.0.0 就改成 127.0.0.1
#[tauri::command]
fn backend_base_url(state: tauri::State<'_, Arc<Desktop>>) -> String {
    let port = state.bind.rsplit(':').next().unwrap_or("8080");
    format!("http://127.0.0.1:{port}")
}

/// Tauri updater：Linux 要照 distro 抓對應的 .deb（20.04 自編棧與 22.04+ 系統棧的 .deb 不能互換），
/// 所以 target 改成 `linux-x86_64-ubuntu-20.04` 這種帶 distro 的鍵，發版時 latest.json 每個 distro 各一筆。
/// macOS／Windows 維持外掛預設的 `darwin-aarch64`／`windows-x86_64`。
fn tauri_updater<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R, tauri_plugin_updater::Config> {
    let mut b = tauri_plugin_updater::Builder::new();
    if cfg!(target_os = "linux") {
        match updater::platform_tag() {
            Some(t) => b = b.target(t),
            None => tracing::warn!("認不出 Linux distro，桌面自動更新會找不到對應的安裝包"),
        }
    }
    b.build()
}

pub fn run(config: Option<PathBuf>, data_dir: Option<PathBuf>) {
    let cancel = CancellationToken::new();
    let cancel_for_exit = cancel.clone();

    tauri::Builder::default()
        // 第二個實例只把既有視窗拉到前面，不會再開一份去搶埠位
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec![])))
        .plugin(tauri_updater())
        .plugin(tauri_plugin_process::init())
        .setup(move |tauri_app| {
            let handle = tauri_app.handle().clone();
            let base = handle.path().app_data_dir().map_err(|e| anyhow::anyhow!("取不到應用資料夾: {e}"))?;
            let (default_cfg, default_data) = app::default_paths(&base);
            let config_path = config.clone().unwrap_or(default_cfg);
            let data_dir = data_dir.clone().unwrap_or(default_data);
            std::fs::create_dir_all(&data_dir)?;
            tracing::info!(config = %config_path.display(), data = %data_dir.display(), "桌面模式啟動");

            // 同步等 bootstrap 完成再開放視窗，前端一開就有 API 可打
            let started = tauri::async_runtime::block_on(app::bootstrap(&config_path, &data_dir, cancel.clone()))
                .map_err(|e| {
                    tracing::error!(?e, "應用啟動失敗");
                    Box::<dyn std::error::Error>::from(e.to_string())
                })?;
            let desktop = Arc::new(Desktop { app: started.app.clone(), bind: started.bind.clone() });
            handle.manage(desktop);

            // 網頁伺服器：埠位被舊版服務占住時每 3 秒重試，視窗照開、畫面會顯示連不上，
            // 而不是整個程式直接崩潰
            let app_state = started.app.clone();
            let bind = started.bind.clone();
            let cancel = cancel.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match server::serve(app_state.clone(), &bind, cancel.clone()).await {
                        Ok(()) => break,
                        Err(e) => {
                            tracing::error!(%bind, "網頁伺服器啟動失敗，3 秒後重試: {e}");
                            tokio::select! {
                                _ = cancel.cancelled() => break,
                                _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => {}
                            }
                        }
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![backend_base_url])
        .build(tauri::generate_context!())
        .expect("建立 Tauri 應用失敗")
        .run(move |_handle, event| {
            // 視窗關閉＝整個程式結束：先通知所有 task 收線，裝置連線才會乾淨關掉
            if let tauri::RunEvent::Exit = event {
                cancel_for_exit.cancel();
            }
        });
}
