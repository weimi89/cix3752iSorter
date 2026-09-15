//! 裝置層：每個裝置一個 task 擁有自己的連線，對外只以 channel 溝通。

pub mod belt;
pub mod camera;
pub mod events;
pub mod line_client;
pub mod sorter;

pub use events::{Device, DeviceEvent};
pub use line_client::{LineClient, LineEvent, LineOpts};

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;

/// 從整份設定的 watch 衍生出「只有某個位址」的 watch：位址沒變就不通知，
/// 裝置 task 才不會因為使用者改了無關設定而重連。
pub fn derive_addr(
    mut cfg_rx: watch::Receiver<AppConfig>,
    pick: impl Fn(&AppConfig) -> String + Send + 'static,
    cancel: CancellationToken,
) -> watch::Receiver<String> {
    let initial = pick(&cfg_rx.borrow());
    let (tx, rx) = watch::channel(initial);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                r = cfg_rx.changed() => {
                    if r.is_err() { break; }
                    let next = pick(&cfg_rx.borrow());
                    if *tx.borrow() != next {
                        let _ = tx.send(next);
                    }
                }
            }
        }
    });
    rx
}
