//! 分揀機裝置 task。連上後先送重置指令（舊系統同樣做法）。

use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use super::events::{Device, DeviceEvent};
use super::line_client::{self, LineClient, LineEvent, LineOpts};
use crate::config::AppConfig;
use crate::protocol::{command, parse_sorter};

pub fn spawn(
    cfg_rx: watch::Receiver<AppConfig>,
    out: mpsc::Sender<DeviceEvent>,
    cancel: CancellationToken,
) -> LineClient {
    let addr_rx = super::derive_addr(cfg_rx, |c| c.sorter.addr.clone(), cancel.clone());
    // 分揀機停線時一句話都不說，沒有探詢會每到讀逾時（5 分鐘）就被當斷線重連一次，
    // 整晚重連幾百次、每次還重置分揀機與亮告警燈。閒置 60 秒就送光電狀態查詢（唯讀，回 ~[…]）當心跳
    let opts = LineOpts {
        on_connect: line_client::fixed_on_connect(vec![command::reset_sorter().to_string()]),
        idle_probe: Some((std::time::Duration::from_secs(60), crate::protocol::ir::query_status().to_string())),
        ..Default::default()
    };
    let (client, mut events) = line_client::spawn("sorter", addr_rx, opts, cancel.clone());

    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            let e = match ev {
                LineEvent::Connected { addr } => {
                    DeviceEvent::State { device: Device::Sorter, connected: true, detail: addr, ts_ms: crate::db::now_ms() }
                }
                LineEvent::Disconnected { addr, reason } => DeviceEvent::State {
                    device: Device::Sorter,
                    connected: false,
                    detail: format!("{addr}: {reason}"),
                    ts_ms: crate::db::now_ms(),
                },
                LineEvent::Line { text, ts_ms } => match parse_sorter(&text) {
                    Some(sig) => DeviceEvent::Sorter { sig, raw: text, ts_ms },
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
