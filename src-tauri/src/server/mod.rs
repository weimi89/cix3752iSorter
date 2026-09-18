//! 網頁後台：axum REST + SSE + 內嵌前端。
//!
//! 存取控制見 `auth.rs`：內網免登入，外網要共用密碼（預設不對外開放）。

mod assets;
mod auth;
mod events;
mod routes;
mod stats;

use std::net::SocketAddr;

use axum::{
    Router,
    response::IntoResponse,
    routing::{get, post},
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::AppState;

#[derive(Clone)]
pub struct ServerState {
    pub app: AppState,
    /// 通知所有 SSE 連線收線（見 events.rs）
    pub close_tx: broadcast::Sender<()>,
}

pub fn router(app: AppState, close_tx: broadcast::Sender<()>) -> Router {
    let state = ServerState { app, close_tx };
    Router::new()
        .nest("/api", routes::api_router())
        .route("/events/stream", get(events::events_stream))
        // 手機遙控：現場人員站在線邊用的簡單頁（獨立 HTML），只有皮帶啟停與狀態，不碰設定
        .route("/control", get(control_page))
        // 登入相關：自身不能被登入中介層擋住，否則外網永遠登不進來
        .route("/auth/status", get(auth::status))
        // 登入是唯一不用登入就能送內容的端點，內容只該有一組密碼：
        // 上限縮到 4 KiB，免得外網用超大 JSON 灌記憶體、再讓 argon2 慢慢算
        .route("/auth/login", post(auth::login).layer(axum::extract::DefaultBodyLimit::max(4 * 1024)))
        .route("/auth/logout", post(auth::logout))
        .fallback(assets::serve)
        // 存取控制：內網放行、外網要密碼、跨站請求一律擋（見 auth.rs）
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth::guard))
        // 最大的內容是整份設定（含列印 profile）與格口表，遠小於 1 MiB
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
        // 桌面模式的視窗來源是 tauri://localhost（Windows 為 http://tauri.localhost），
        // 開發時是 Vite 的 localhost；只放行這些，LAN 上其他網頁不能跨站打 API
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::predicate(|origin, _| auth::is_trusted_origin(origin.as_bytes())))
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// 跑到 `cancel` 觸發為止；先叫 SSE 收線，再讓 axum 優雅關閉。
pub async fn serve(app: AppState, bind: &str, cancel: CancellationToken) -> anyhow::Result<()> {
    let (close_tx, _) = broadcast::channel::<()>(1);
    let router = router(app, close_tx.clone());
    let addr: SocketAddr = bind.parse().map_err(|e| anyhow::anyhow!("server.bind `{bind}` 格式錯誤: {e}"))?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "網頁後台啟動");

    // into_make_service_with_connect_info：讓中介層取得 TCP 對端位址。
    // 內外網的判斷完全依賴它 —— 少了這行，ConnectInfo 抽取失敗會讓所有請求被擋。
    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move {
            cancel.cancelled().await;
            let _ = close_tx.send(());
        })
        .await?;
    Ok(())
}

async fn control_page() -> impl IntoResponse {
    axum::response::Html(include_str!("control_page.html"))
}
