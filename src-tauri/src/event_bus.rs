//! 事件匯流排：後端各模組發事件，網頁端經 SSE 訂閱。
//!
//! 刻意做成進程唯一的靜態值：事件不屬於任何一份狀態，綁進 AppState 只會讓
//! 每個 emit 呼叫點都多拿一個參數。**所有推給前端的事件都走這裡**，
//! 不要各處自開 channel，否則新增的事件會漏推。

use once_cell::sync::Lazy;
use serde::Serialize;
use tokio::sync::broadcast;

#[derive(Clone, Debug, Serialize)]
pub struct BusEvent {
    pub event: String,
    pub payload: serde_json::Value,
}

/// 容量 256：尖峰是連續刷件（每件數則事件）；網頁端跟不上時走 `Lagged` 丟舊事件，不阻塞發送端。
static BUS: Lazy<broadcast::Sender<BusEvent>> = Lazy::new(|| broadcast::channel(256).0);

pub fn subscribe() -> broadcast::Receiver<BusEvent> {
    BUS.subscribe()
}

/// 沒有訂閱者時 `send` 回 Err，那是正常狀態（沒人開網頁），不視為錯誤。
pub fn emit<S: Serialize>(event: &str, payload: S) {
    let json = serde_json::to_value(&payload).unwrap_or(serde_json::Value::Null);
    let _ = BUS.send(BusEvent { event: event.to_string(), payload: json });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn 訂閱者收得到送出的事件() {
        let mut rx = subscribe();
        emit("device-state", serde_json::json!({ "device": "belt", "connected": true }));
        let got = rx.recv().await.expect("應收到事件");
        assert_eq!(got.event, "device-state");
        assert_eq!(got.payload["device"], "belt");
    }

    #[test]
    fn 沒有訂閱者時發送不會恐慌() {
        emit("system-message", "x");
    }
}
