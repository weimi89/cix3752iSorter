//! 皮帶線裝置 task：連線由 `line_client` 管，這裡只做解析與轉發。
//! 連上後先送「停止」再送「重置」（舊系統同樣做法）：程式重啟或斷線重連時皮帶不會帶著舊狀態自己動。

use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use super::events::{Device, DeviceEvent};
use super::line_client::{self, LineClient, LineEvent, LineOpts};
use crate::config::AppConfig;
use crate::protocol::parse_belt;

pub fn spawn(
    cfg_rx: watch::Receiver<AppConfig>,
    out: mpsc::Sender<DeviceEvent>,
    cancel: CancellationToken,
) -> LineClient {
    let addr_rx = super::derive_addr(cfg_rx.clone(), |c| c.belt.addr.clone(), cancel.clone());
    let on_connect = super::derive(cfg_rx, |c| vec![c.belt.cmd.stop.clone(), c.belt.cmd.reset.clone()], cancel.clone());
    // 皮帶線沒有件時整段不說話（運轉中也一樣），又沒有唯讀查詢可當心跳；照 300 秒讀逾時會把閒置當斷線，
    // 重連時的「先停止再重置」還會真的把皮帶停下來（2026-09-18 19:50 現場就是這樣停的）。
    // 斷線只靠 TCP keepalive（10 秒沒回 ACK 開始探、5 秒一次）抓：控制器活著就不會被誤判
    let opts = LineOpts { on_connect, idle_timeout: None, ..Default::default() };
    let (client, mut events) = line_client::spawn("belt", addr_rx, opts, cancel.clone());

    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            let e = match ev {
                LineEvent::Connected { addr } => {
                    DeviceEvent::State { device: Device::Belt, connected: true, detail: addr, ts_ms: crate::db::now_ms() }
                }
                LineEvent::Disconnected { addr, reason } => DeviceEvent::State {
                    device: Device::Belt,
                    connected: false,
                    detail: format!("{addr}: {reason}"),
                    ts_ms: crate::db::now_ms(),
                },
                LineEvent::Line { text, ts_ms } => match parse_belt(&text) {
                    Some(sig) => DeviceEvent::Belt { sig, raw: text, ts_ms },
                    None => continue,
                },
            };
            if out.send(e).await.is_err() {
                break;
            }
        }
    });

    client
}
