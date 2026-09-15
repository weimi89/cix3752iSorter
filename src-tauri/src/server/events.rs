//! `GET /events/stream` —— 把事件匯流排轉成 SSE 給網頁端。
//!
//! 前端所有頁面共用一條連線（`frontend/src/api/events.js` 在瀏覽器端分發），
//! `?only=a,b` 可只收指定事件。

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    response::sse::{Event, KeepAlive, Sse},
};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::event_bus::{self, BusEvent};

#[derive(Debug, Deserialize)]
pub(super) struct StreamQuery {
    #[serde(default)]
    only: Option<String>,
}

pub(super) async fn events_stream(
    State(state): State<super::ServerState>,
    Query(q): Query<StreamQuery>,
) -> impl IntoResponse {
    let filter: Option<Vec<String>> = q.only.map(|s| {
        s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
    });

    sse_from(event_bus::subscribe(), state.close_tx.subscribe(), move |ev| {
        let wanted = filter.as_ref().is_none_or(|f| f.contains(&ev.event));
        wanted.then(|| Event::default().json_data(ev).ok()).flatten()
    })
}

/// SSE 本身永遠不會自己結束，而 axum 的 graceful shutdown 會等所有連線收工 ——
/// `close_rx` 收到通知就主動收線，否則只要有人開著網頁，服務就關不掉。
fn sse_from<F>(
    rx: broadcast::Receiver<BusEvent>,
    close_rx: broadcast::Receiver<()>,
    pick: F,
) -> impl IntoResponse
where
    F: Fn(&BusEvent) -> Option<Event> + Send + Sync + 'static,
{
    let pick = std::sync::Arc::new(pick);
    let stream = futures::stream::unfold((rx, close_rx), move |(mut rx, mut close_rx)| {
        let pick = pick.clone();
        async move {
            loop {
                tokio::select! {
                    _ = close_rx.recv() => return None,
                    got = rx.recv() => match got {
                        Ok(ev) => {
                            if let Some(sse) = pick(&ev) {
                                return Some((Ok::<_, std::convert::Infallible>(sse), (rx, close_rx)));
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
