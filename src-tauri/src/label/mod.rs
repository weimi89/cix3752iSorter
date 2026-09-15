//! 面單列印流程：狀態機接受格口決定後把已下載的面單交過來 → 點陣 → TSPL → 列印佇列。
//! 面單在格口解析階段就抓好了（抓不到的件不會走到這裡），這裡不再碰網路。

pub mod queue;
pub mod raster;
pub mod tspl;
pub mod usb;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::AppState;
use crate::event_log::{self, Level};
use crate::tracker::LabelPayload;

pub use queue::PrintService;

/// 一件要印的面單
#[derive(Clone, Debug)]
pub struct LabelJob {
    pub parcel_ulid: String,
    pub barcode: String,
    pub chute_code: String,
    pub printer_port: Option<String>,
    pub label: LabelPayload,
}

/// 格口代號裡的數字（L3 → 3），送印延遲用；沒有數字視為 1
pub fn chute_number(code: &str) -> u32 {
    let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse().unwrap_or(1).max(1)
}

pub fn spawn_pipeline(app: AppState, printer: PrintService, mut rx: mpsc::Receiver<LabelJob>, cancel: CancellationToken) {
    tokio::spawn(async move {
        loop {
            let job = tokio::select! {
                _ = cancel.cancelled() => break,
                r = rx.recv() => match r { Some(j) => j, None => break },
            };
            let app = app.clone();
            let printer = printer.clone();
            tokio::spawn(async move {
                if let Err(e) = handle(&app, &printer, &job).await {
                    event_log::log(&app.db, Level::Error, "printer", "label", format!("{} 條碼 {} 面單處理失敗: {e}", job.chute_code, job.barcode));
                }
            });
        }
    });
}

async fn handle(app: &AppState, printer: &PrintService, job: &LabelJob) -> anyhow::Result<()> {
    let Some(port) = job.printer_port.as_deref() else {
        tracing::info!(chute = %job.chute_code, barcode = %job.barcode, "格口未設印表機，不列印");
        return Ok(());
    };
    let started = std::time::Instant::now();
    let cfg = app.config.current();
    let profile_name = job.label.print_profile.clone();
    let profile = profile_name.as_deref().and_then(|n| cfg.print.profiles.get(n)).cloned().unwrap_or_default();
    let pn = profile_name.clone();
    let bytes = job.label.bytes.clone();
    let tspl = tokio::task::spawn_blocking(move || -> anyhow::Result<(Vec<u8>, u32, u32)> {
        let r = raster::render(&bytes, pn.as_deref())?;
        Ok((tspl::build(&r, &profile), r.width, r.height))
    })
    .await??;
    let delay_ms = (chute_number(&job.chute_code) as i64 - 1) * cfg.print.per_chute_delay_ms as i64;
    let id = printer
        .enqueue(queue::NewJob {
            parcel_ulid: Some(&job.parcel_ulid),
            barcode: &job.barcode,
            chute_code: &job.chute_code,
            printer_port: port,
            profile: profile_name.as_deref(),
            tspl: &tspl.0,
            delay_ms,
        })
        .await?;
    tracing::info!(job = id, chute = %job.chute_code, barcode = %job.barcode, px = format!("{}x{}", tspl.1, tspl.2), ms = started.elapsed().as_millis() as u64, delay_ms, "面單已入列");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn 格口號() {
        assert_eq!(super::chute_number("L3"), 3);
        assert_eq!(super::chute_number("R1"), 1);
        assert_eq!(super::chute_number("LS"), 1);
    }
}
