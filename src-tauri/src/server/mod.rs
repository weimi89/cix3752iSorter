//! 網頁後台：axum REST + SSE + 內嵌前端。

mod assets;
mod events;
mod routes;

use std::net::SocketAddr;

use axum::{Router, routing::get};
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
        .fallback(assets::serve)
        // 更新用的 tar.gz 約 20MB，預設 2MB 上限會擋掉
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024))
        // 桌面模式的視窗來源是 tauri://localhost（Windows 為 http://tauri.localhost），
        // 開發時是 Vite 的 localhost；只放行這些，LAN 上其他網頁不能跨站打 API
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::predicate(|origin, _| {
                    let o = origin.as_bytes();
                    o == b"tauri://localhost" || o == b"http://tauri.localhost" || o.starts_with(b"http://localhost:") || o.starts_with(b"http://127.0.0.1:")
                }))
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

    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move {
            cancel.cancelled().await;
            let _ = close_tx.send(());
        })
        .await?;
    Ok(())
}
