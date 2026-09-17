//! `GET /events/stream` —— 把事件匯流排轉成 SSE 給網頁端。
//!
//! 前端所有頁面共用一條連線（`frontend/src/api/events.js` 在瀏覽器端分發），
//! `?only=a,b` 可只收指定事件。

use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    response::sse::{Event, KeepAlive, Sse},
};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::event_bus::{self, BusEvent};

/// 多久回頭確認一次這條串流還能不能收（內外網與 session 都重算，見 auth::stream_still_allowed）
const RECHECK_SECS: u64 = 60;

#[derive(Debug, Deserialize)]
pub(super) struct StreamQuery {
    #[serde(default)]
    only: Option<String>,
}

pub(super) async fn events_stream(
    State(state): State<super::ServerState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<StreamQuery>,
) -> impl IntoResponse {
    let recheck = (state.clone(), peer.ip(), super::auth::stream_token(&headers));
    let filter: Option<Vec<String>> = q.only.map(|s| {
        s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
    });

    sse_from(event_bus::subscribe(), state.close_tx.subscribe(), recheck, move |ev| {
        let wanted = filter.as_ref().is_none_or(|f| f.contains(&ev.event));
        wanted.then(|| Event::default().json_data(ev).ok()).flatten()
    })
}

/// SSE 本身永遠不會自己結束，而 axum 的 graceful shutdown 會等所有連線收工 ——
/// `close_rx` 收到通知就主動收線，否則只要有人開著網頁，服務就關不掉。
/// `recheck` 每隔 `RECHECK_SECS` 回頭確認這條連線還有沒有資格收（外網 session 逾期就收線）。
fn sse_from<F>(
    rx: broadcast::Receiver<BusEvent>,
    close_rx: broadcast::Receiver<()>,
    recheck: (super::ServerState, std::net::IpAddr, String),
    pick: F,
) -> impl IntoResponse
where
    F: Fn(&BusEvent) -> Option<Event> + Send + Sync + 'static,
{
    let pick = std::sync::Arc::new(pick);
    let recheck = std::sync::Arc::new(recheck);
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(RECHECK_SECS));
    ticker.reset();
    let stream = futures::stream::unfold((rx, close_rx, ticker), move |(mut rx, mut close_rx, mut ticker)| {
        let pick = pick.clone();
        let recheck = recheck.clone();
        async move {
            loop {
                tokio::select! {
                    _ = close_rx.recv() => return None,
                    _ = ticker.tick() => {
                        let (st, ip, token) = recheck.as_ref();
                        if !super::auth::stream_still_allowed(st, *ip, token).await {
                            tracing::info!(%ip, "事件串流已不再被允許，收線");
                            return None;
                        }
                    }
                    got = rx.recv() => match got {
                        Ok(ev) => {
                            if let Some(sse) = pick(&ev) {
                                return Some((Ok::<_, std::convert::Infallible>(sse), (rx, close_rx, ticker)));
                            }
                        }
                        // 落後時只補最新的，舊事件直接丟 —— 現場要看的是「現在這件」
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return None,
                    },
                }
            }
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
