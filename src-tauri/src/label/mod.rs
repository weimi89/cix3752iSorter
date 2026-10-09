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

/// 登記在結束流程要等的工作裡：收到結束通知後不再收新的面單，但已經送進來的都要寫進列印佇列
/// （寫進去的重開後會接著印；還在記憶體裡的關掉就沒了，那幾件就少一張面單）
pub fn spawn_pipeline(app: AppState, printer: PrintService, mut rx: mpsc::Receiver<LabelJob>, cancel: CancellationToken) {
    let tasks = app.tasks.clone();
    crate::supervise::spawn_tracked("面單處理", &tasks, async move {
        loop {
            let job = tokio::select! {
                biased;
                r = rx.recv() => match r { Some(j) => j, None => break },
                _ = cancel.cancelled() => {
                    rx.close();
                    continue;
                }
            };
            // 一張做完才做下一張：列印佇列照入列先後取件，各自並行的話點陣小的那張會先入列，
            // 同一格口前後兩件的面單就對調了。點陣在正式版一張幾十毫秒，依序做跟得上每秒一件
            if let Err(e) = handle(&app, &printer, &job).await {
                event_log::log(&app.db, Level::Error, "printer", "label", format!("{} 條碼 {} 面單處理失敗: {e}", job.chute_code, job.barcode));
            }
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

    /// 隔離的程式狀態與面單處理流程；回傳送面單的入口
    async fn pipeline() -> (crate::AppState, tokio::sync::mpsc::Sender<super::LabelJob>, tokio_util::sync::CancellationToken) {
        use crate::config::{AppConfig, ConfigHandle};
        let dir = std::env::temp_dir().join(format!("labelorder-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::init(&dir).await.unwrap();
        let mut cfg = AppConfig::default();
        cfg.middleware.base_url = "http://127.0.0.1:1/".into(); // 告警不要真的送出去
        let config = ConfigHandle::new(dir.join("config.toml"), cfg);
        let app = crate::AppState {
            config: config.clone(),
            db: db.clone(),
            read_db: db.clone(),
            data_dir: dir.clone(),
            started_at: std::time::Instant::now(),
            runtime: crate::runtime::Runtime::default(),
            tracker: Default::default(),
            resolver: Default::default(),
            printer: Default::default(),
            tasks: tokio_util::task::TaskTracker::new(),
        };
        let cancel = tokio_util::sync::CancellationToken::new();
        let svc = super::PrintService::new(db.clone(), config.subscribe(), crate::middleware::Middleware::new(config.subscribe()), &dir, cancel.clone(), app.tasks.clone());
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        super::spawn_pipeline(app.clone(), svc, rx, cancel.clone());
        (app, tx, cancel)
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::GrayImage::from_pixel(w, h, image::Luma([255u8]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(img).write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    fn job(barcode: &str, bytes: Vec<u8>) -> super::LabelJob {
        super::LabelJob {
            parcel_ulid: ulid::Ulid::generate().to_string(),
            barcode: barcode.into(),
            chute_code: "L1".into(),
            printer_port: Some("8-8.8".into()), // 找不到這台：只入列不會真的送印
            label: crate::tracker::LabelPayload { bytes, print_profile: Some("PAPER-01#100*150".into()), is_error_label: false },
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn 關閉時已送來的面單_都要寫進列印佇列才結束() {
        let (app, tx, cancel) = pipeline().await;
        for i in 0..3 {
            tx.send(job(&format!("SHUT{i}"), png(1600, 2400))).await.unwrap();
        }
        // 第一張還在產生點陣時就關閉
        cancel.cancel();
        assert!(crate::supervise::drain(&app.tasks, std::time::Duration::from_secs(60)).await);
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM print_jobs").fetch_one(&app.db).await.unwrap();
        assert_eq!(n, 3, "已經交給列印流程的面單不可在關閉時丟掉");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn 同一台印表機的面單_入列順序要跟送來的順序一樣() {
        let (app, tx, cancel) = pipeline().await;
        let db = app.db.clone();
        // 先到的那張面單比較大、產生點陣比較久；後到的小張不可插到前面去
        tx.send(job("FIRST", png(1600, 2400))).await.unwrap();
        tx.send(job("SECOND", png(400, 600))).await.unwrap();
        let order: Vec<String> = tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                let v: Vec<(String,)> = sqlx::query_as("SELECT barcode FROM print_jobs ORDER BY id").fetch_all(&db).await.unwrap();
                if v.len() == 2 {
                    return v.into_iter().map(|r| r.0).collect();
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("兩張都要入列");
        cancel.cancel();
        assert_eq!(order, vec!["FIRST", "SECOND"]);
    }

}
