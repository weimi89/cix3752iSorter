//! 裝置模擬器：回放舊系統的 `cmd.log`，扮演皮帶線、分揀機（TCP server）與相機（TCP client）。
//!
//! 主程式照正式設定連過來就能跑完整流程，不需要實機。
//!
//!   cargo run --bin sorter-replay -- --log ../main_proj/data/cmd.log --speed 20 --from "09-10 18:00" --to "09-10 19:00"
//!
//! 主程式的 `config.toml` 要指到模擬器：belt.addr = 127.0.0.1:17100、sorter.addr = 127.0.0.1:17198，
//! 相機監聽維持 0.0.0.0:8051（模擬器會連進去）。
//!
//! `--drop-every 30` 每 30 秒隨機掛斷一條裝置連線，用來驗證主程式的重連。

use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, mpsc};

#[derive(Parser, Debug)]
#[command(about = "回放 cmd.log 的裝置模擬器")]
struct Cli {
    #[arg(long)]
    log: String,
    #[arg(long, default_value = "127.0.0.1:17100")]
    belt: String,
    #[arg(long, default_value = "127.0.0.1:17198")]
    sorter: String,
    /// 主程式的相機監聽位址（模擬器當相機連過去）
    #[arg(long, default_value = "127.0.0.1:8051")]
    camera: String,
    /// 回放倍速
    #[arg(long, default_value_t = 1.0)]
    speed: f64,
    /// 只回放此時間之後（`MM-DD HH:MM`，比對日誌行首）
    #[arg(long)]
    from: Option<String>,
    #[arg(long)]
    to: Option<String>,
    #[arg(long)]
    max_lines: Option<usize>,
    /// 每 N 秒隨機掛斷一條裝置連線（0 = 不掛斷）
    #[arg(long, default_value_t = 0u64)]
    drop_every: u64,
    /// 不印主程式送來的指令
    #[arg(long, default_value_t = false)]
    quiet: bool,
    /// `~c` 閘門：每收到主程式一個 Kn 才放行一個 `~c`（維持因果；高倍速回放必開）
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    gate_c: bool,
    /// 假中介機監聽位址（主程式 config.toml 的 middleware.base_url 指到這裡）
    #[arg(long, default_value = "127.0.0.1:18081")]
    middleware: String,
    /// 假中介機回應延遲中位數（毫秒，實際為 0.6–1.6 倍抖動）
    #[arg(long, default_value_t = 350u64)]
    mw_delay_ms: u64,
    /// 假中介機每 N 件回一次「查無訂單」業務錯誤（0 = 不回）
    #[arg(long, default_value_t = 0u64)]
    mw_error_every: u64,
    /// 假中介機每 N 張面單圖回 500（0 = 不失敗）：驗「面單抓不到 → 走預設口、不回報、不印」
    #[arg(long, default_value_t = 0u64)]
    mw_img_fail_every: u64,
    /// 假中介機面單圖回應延遲（毫秒）：拉長可驗「面單太晚 → 走預設口」
    #[arg(long, default_value_t = 0u64)]
    mw_img_delay_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Src {
    Belt,
    Sorter,
    Camera,
}

struct Entry {
    ts_ms: i64,
    src: Src,
    line: String,
}

/// 只留裝置「送出」的行；主程式自己記的（Kn、Init Pac、Handling…）不回放
fn parse_entry(raw: &str) -> Option<Entry> {
    // 格式：MM-DD HH:MM:SS.mmm, <ts_ms>, [src] [idx] > msg
    let (head, msg) = raw.split_once(" > ")?;
    let mut parts = head.splitn(3, ", ");
    let _human = parts.next()?;
    let ts_ms: i64 = parts.next()?.trim().parse().ok()?;
    let tag = parts.next()?;
    let src = if tag.starts_with("[belt]") {
        Src::Belt
    } else if tag.starts_with("[sorter]") {
        Src::Sorter
    } else if tag.starts_with("[camera]") {
        Src::Camera
    } else {
        return None;
    };
    let msg = msg.trim();
    let keep = match src {
        Src::Belt | Src::Sorter => msg.starts_with('~') || msg.contains("<<<") || msg.contains(" = ") || msg.contains("FFFFFFFF"),
        Src::Camera => {
            !(msg.starts_with("handle camera data") || msg.starts_with("pac ") || msg.starts_with("camera data") || msg.is_empty())
        }
    };
    keep.then(|| Entry { ts_ms, src, line: msg.to_string() })
}

/// 把日誌裡「已挑好的條碼」包回相機幀格式；NoRead 送空幀
fn camera_frame(code: &str) -> String {
    if code == "NoRead" { "@".to_string() } else { format!("{code};(0,0)(0,0)(0,0)(0,0);Code128@") }
}

/// 一個裝置 server：同一時間只服務一個 client；`tx` 送來的行寫給 client，client 送來的指令印出
struct DeviceServer {
    name: &'static str,
    conn: Arc<Mutex<Option<tokio::net::tcp::OwnedWriteHalf>>>,
    stats: Arc<Mutex<(u64, u64)>>, // (sent, received)
    /// 收到的 Kn 數（`~c` 閘門用：每收到一個 Kn 才放行一個 `~c`）
    kn_seen: Arc<Mutex<u64>>,
}

impl DeviceServer {
    async fn start(name: &'static str, bind: String, echo: bool) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(&bind).await?;
        eprintln!("[{name}] 監聽 {bind}");
        let conn: Arc<Mutex<Option<tokio::net::tcp::OwnedWriteHalf>>> = Arc::new(Mutex::new(None));
        let stats = Arc::new(Mutex::new((0u64, 0u64)));
        let kn_seen = Arc::new(Mutex::new(0u64));
        let c2 = conn.clone();
        let s2 = stats.clone();
        let k2 = kn_seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, peer)) = listener.accept().await else { continue };
                eprintln!("[{name}] 主程式連線 {peer}");
                let _ = stream.set_nodelay(true);
                let (rd, wr) = stream.into_split();
                *c2.lock().await = Some(wr);
                let mut lines = tokio::io::BufReader::new(rd).lines();
                let s3 = s2.clone();
                while let Ok(Some(l)) = lines.next_line().await {
                    s3.lock().await.1 += 1;
                    if l.trim_start().starts_with("Kn ") {
                        *k2.lock().await += 1;
                    }
                    if echo {
                        eprintln!("[{name}] <- {}", l.trim());
                    }
                }
                eprintln!("[{name}] 主程式斷線 {peer}");
                *c2.lock().await = None;
            }
        });
        Ok(Self { name, conn, stats, kn_seen })
    }

    async fn send(&self, line: &str) {
        let mut g = self.conn.lock().await;
        if let Some(wr) = g.as_mut() {
            if wr.write_all(format!("{line}\n").as_bytes()).await.is_err() {
                *g = None;
            } else {
                self.stats.lock().await.0 += 1;
            }
        }
    }

    async fn drop_conn(&self) {
        if self.conn.lock().await.take().is_some() {
            eprintln!("[{}] 模擬掛斷", self.name);
        }
    }
}

/// 相機 client：連不上就一直重試；送幀失敗就重連
async fn camera_client(addr: String, mut rx: mpsc::Receiver<String>) {
    let mut stream: Option<TcpStream> = None;
    while let Some(frame) = rx.recv().await {
        for _ in 0..3 {
            if stream.is_none() {
                match TcpStream::connect(&addr).await {
                    Ok(s) => {
                        eprintln!("[camera] 連上主程式 {addr}");
                        stream = Some(s);
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        continue;
                    }
                }
            }
            if let Some(s) = stream.as_mut() {
                if s.write_all(frame.as_bytes()).await.is_ok() {
                    break;
                }
                eprintln!("[camera] 寫入失敗，重連");
                stream = None;
            }
        }
    }
}

/// 假中介機：照 `cix3752iLabelPrint/docs/local-http-api.md` 的形狀回應
async fn fake_middleware(bind: String, delay_ms: u64, error_every: u64, img_fail_every: u64, img_delay_ms: u64) -> anyhow::Result<()> {
    use axum::{Router, extract::Path, routing::{get, post}, Json};
    use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

    static NEXT_ID: AtomicI64 = AtomicI64::new(1);
    static SEEN: AtomicU64 = AtomicU64::new(0);
    static REPORTS: AtomicU64 = AtomicU64::new(0);
    static NOREADS: AtomicU64 = AtomicU64::new(0);
    static IMAGES: AtomicU64 = AtomicU64::new(0);

    let label_png: Arc<Vec<u8>> = Arc::new({
        // 800x1200 白底、黑框、幾條黑線，當作面單
        let mut img = image::GrayImage::from_pixel(800, 1200, image::Luma([255u8]));
        for x in 0..800 { for y in [0usize, 1, 1198, 1199] { img.put_pixel(x, y as u32, image::Luma([0])); } }
        for y in 0..1200 { for x in [0u32, 1, 798, 799] { img.put_pixel(x, y, image::Luma([0])); } }
        for y in (100..1100).step_by(100) { for x in 50..750 { img.put_pixel(x, y, image::Luma([0])); } }
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(img).write_to(&mut buf, image::ImageFormat::Png)?;
        buf.into_inner()
    });

    let bind2 = bind.clone();
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/parcel/{code}", get(move |Path(code): Path<String>| {
            let bind = bind2.clone();
            async move {
                let jitter = 0.6 + (SEEN.fetch_add(1, Ordering::Relaxed) % 11) as f64 / 10.0; // 0.6–1.6
                tokio::time::sleep(Duration::from_millis((delay_ms as f64 * jitter) as u64)).await;
                if code.eq_ignore_ascii_case("noread") {
                    let n = NOREADS.fetch_add(1, Ordering::Relaxed) + 1;
                    eprintln!("[mw] NoRead 通知 #{n}");
                    return Json(serde_json::json!({ "data": { "channel_code": null, "print_profile": null, "response_id": null, "error_code": "NOREAD", "message": "讀碼失敗,未提交雲端" } }));
                }
                let n = SEEN.load(Ordering::Relaxed);
                if error_every > 0 && n % error_every == 0 {
                    return Json(serde_json::json!({ "data": { "channel_code": null, "print_profile": null, "response_id": null, "is_error_label": false, "error_code": "NOT_FOUND", "message": "查無訂單" } }));
                }
                let chutes = ["L1", "L2", "L3", "L4", "L5", "R1", "R2", "R3", "R4", "R5", "LS"];
                let h = code.bytes().fold(7u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64));
                let chute = chutes[(h % chutes.len() as u64) as usize];
                let rid = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                let mut data = serde_json::json!({ "channel_code": chute, "print_profile": "PAPER-01#100*150", "response_id": rid });
                if chute != "LS" {
                    data["label_path"] = serde_json::Value::String(format!("http://{bind}/images/{code}.png"));
                }
                Json(serde_json::json!({ "data": data }))
            }
        }))
        .route("/images/{name}", get(move |Path(name): Path<String>| {
            let png = label_png.clone();
            async move {
                if img_delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(img_delay_ms)).await;
                }
                let n = IMAGES.fetch_add(1, Ordering::Relaxed) + 1;
                if img_fail_every > 0 && n % img_fail_every == 0 {
                    eprintln!("[mw] 面單圖 {name} 故意回 500（第 {n} 張）");
                    return Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
                }
                Ok(([(axum::http::header::CONTENT_TYPE, "image/png")], png.as_ref().clone()))
            }
        }))
        .route("/api/report", post(|Json(body): Json<serde_json::Value>| async move {
            let n = REPORTS.fetch_add(1, Ordering::Relaxed) + 1;
            if n % 500 == 0 { eprintln!("[mw] 已收到 {n} 筆回報"); }
            if body.get("response_id").and_then(|v| v.as_i64()).is_none() {
                return (axum::http::StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({ "message": "缺 response_id" })));
            }
            (axum::http::StatusCode::OK, Json(serde_json::json!({ "message": "OK" })))
        }))
        .route("/api/device-alert", post(|Json(body): Json<serde_json::Value>| async move {
            eprintln!("[mw] 設備異常: {body}");
            Json(serde_json::json!({ "message": "OK" }))
        }));
    let listener = TcpListener::bind(&bind).await?;
    eprintln!("[mw] 假中介機監聽 {bind}（延遲中位 {delay_ms}ms）");
    axum::serve(listener, app).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    {
        let (bind, d, e, f, g) = (cli.middleware.clone(), cli.mw_delay_ms, cli.mw_error_every, cli.mw_img_fail_every, cli.mw_img_delay_ms);
        tokio::spawn(async move {
            if let Err(e) = fake_middleware(bind, d, e, f, g).await {
                eprintln!("[mw] 假中介機啟動失敗: {e}");
            }
        });
    }

    let file = std::fs::File::open(&cli.log)?;
    let mut entries = Vec::new();
    let mut in_range = cli.from.is_none();
    for raw in BufReader::new(file).lines().map_while(Result::ok) {
        if let Some(from) = &cli.from {
            if !in_range && raw.starts_with(from) {
                in_range = true;
            }
        }
        if !in_range {
            continue;
        }
        if let Some(to) = &cli.to {
            if raw.starts_with(to) {
                break;
            }
        }
        if let Some(e) = parse_entry(&raw) {
            entries.push(e);
        }
        if cli.max_lines.is_some_and(|m| entries.len() >= m) {
            break;
        }
    }
    if entries.is_empty() {
        anyhow::bail!("沒有可回放的行（檢查 --from/--to 是否符合日誌行首格式 MM-DD HH:MM）");
    }
    let total = entries.len();
    let span_ms = entries.last().unwrap().ts_ms - entries[0].ts_ms;
    eprintln!("載入 {total} 行，涵蓋 {:.1} 分鐘，倍速 {} → 預計 {:.1} 分鐘", span_ms as f64 / 60000.0, cli.speed, span_ms as f64 / 60000.0 / cli.speed);

    let belt = DeviceServer::start("belt", cli.belt.clone(), !cli.quiet).await?;
    let sorter = DeviceServer::start("sorter", cli.sorter.clone(), !cli.quiet).await?;
    let (cam_tx, cam_rx) = mpsc::channel::<String>(1024);
    tokio::spawn(camera_client(cli.camera.clone(), cam_rx));

    // 等主程式連上皮帶與分揀機再開始，否則前面的訊號全部送進空氣
    eprintln!("等待主程式連線…");
    loop {
        if belt.conn.lock().await.is_some() && sorter.conn.lock().await.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;

    let start_real = Instant::now();
    let start_log = entries[0].ts_ms;
    let mut last_drop = Instant::now();
    let mut progress_at = Instant::now();
    // `~c` 是分揀機對「我們的 Kn」的回應：日誌裡的時間點對應舊程式的 Kn，
    // 高倍速回放時會搶在新程式的 Kn 之前抵達而失去因果。這裡把 `~c` 關在閘門後，
    // 每收到一個 Kn 才放行一個，其餘訊號照日誌時序。
    let mut c_released: u64 = 0;
    // 待放行的 (~c 行, 該小車其後被扣住的訊號)
    let mut c_pending: std::collections::VecDeque<(String, Vec<String>)> = std::collections::VecDeque::new();
    fn cart_of(line: &str) -> Option<&str> {
        let rest = line.get(2..)?;
        let id = rest.split_whitespace().next()?;
        id.parse::<u32>().ok().map(|_| id)
    }

    for (i, e) in entries.iter().enumerate() {
        let due = Duration::from_secs_f64((e.ts_ms - start_log) as f64 / 1000.0 / cli.speed);
        loop {
            let elapsed = start_real.elapsed();
            // 等待期間持續檢查閘門，Kn 一到就放行對應的 ~c
            if cli.gate_c && !c_pending.is_empty() && *sorter.kn_seen.lock().await > c_released {
                let (line, held) = c_pending.pop_front().unwrap();
                c_released += 1;
                tokio::time::sleep(Duration::from_secs_f64(0.014 / cli.speed)).await;
                sorter.send(&line).await;
                for h in held {
                    tokio::time::sleep(Duration::from_secs_f64(0.012 / cli.speed)).await;
                    sorter.send(&h).await;
                }
                continue;
            }
            if due <= elapsed {
                break;
            }
            tokio::time::sleep((due - elapsed).min(Duration::from_millis(2))).await;
        }
        match e.src {
            Src::Belt => belt.send(&e.line).await,
            Src::Sorter => {
                if cli.gate_c && e.line.starts_with("~c") {
                    if *sorter.kn_seen.lock().await > c_released {
                        c_released += 1;
                        sorter.send(&e.line).await;
                    } else {
                        c_pending.push_back((e.line.clone(), Vec::new()));
                    }
                } else if cli.gate_c && !c_pending.is_empty() && e.line.starts_with('~') {
                    // 同一台小車在 ~c 之後的訊號（~j/~g/~e/~k/~u/~x）要跟著 ~c 一起等
                    let cart = cart_of(&e.line).map(str::to_string);
                    let held_by = cart.as_deref().and_then(|c| c_pending.iter_mut().rev().find(|(l, _)| cart_of(l) == Some(c)));
                    match held_by {
                        Some((_, held)) if !e.line.starts_with("~q") && !e.line.starts_with("~v") && !e.line.starts_with("~I") => held.push(e.line.clone()),
                        _ => sorter.send(&e.line).await,
                    }
                } else {
                    sorter.send(&e.line).await;
                }
            }
            Src::Camera => {
                let _ = cam_tx.send(camera_frame(&e.line)).await;
            }
        }
        if cli.drop_every > 0 && last_drop.elapsed() >= Duration::from_secs(cli.drop_every) {
            last_drop = Instant::now();
            if i % 2 == 0 { belt.drop_conn().await } else { sorter.drop_conn().await }
        }
        if progress_at.elapsed() >= Duration::from_secs(10) {
            progress_at = Instant::now();
            let (bs, br) = *belt.stats.lock().await;
            let (ss, sr) = *sorter.stats.lock().await;
            eprintln!("進度 {}/{} ({:.0}%)  belt 送{bs}/收{br}  sorter 送{ss}/收{sr}", i + 1, total, (i + 1) as f64 * 100.0 / total as f64);
        }
    }
    // 讓最後幾個訊號的處理跑完
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (bs, br) = *belt.stats.lock().await;
    let (ss, sr) = *sorter.stats.lock().await;
    eprintln!("回放完成：belt 送{bs}/收{br}  sorter 送{ss}/收{sr}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 只回放裝置送出的行() {
        let e = parse_entry("08-31 18:27:17.402, 1788172037402, [belt] [0] > ~P25 1").unwrap();
        assert_eq!((e.ts_ms, e.src, e.line.as_str()), (1788172037402, Src::Belt, "~P25 1"));
        assert!(parse_entry("08-31 18:27:17.412, 1788172037412, [belt] [0] > Init Pac 1788165552").is_none());
        assert!(parse_entry("08-31 18:27:18.733, 1788172038733, [sorter] [0] > [1788165552] Kn 101 01 43 80 24 1").is_none());
        assert!(parse_entry("08-31 18:27:18.733, 1788172038733, [sorter] [0] > Group[0] Handling HeadPac 1 takes 1ms").is_none());
        let c = parse_entry("08-31 18:27:17.618, 1788172037618, [camera] [0] > 99K00064643").unwrap();
        assert_eq!((c.src, c.line.as_str()), (Src::Camera, "99K00064643"));
        assert!(parse_entry("08-31 18:27:17.631, 1788172037631, [camera] [0] > handle camera data 99K00064643 in 13ms").is_none());
        assert!(parse_entry("08-31 18:27:17.631, 1788172037631, [camera] [0] > pac 1 binding camera 99K00064643 used 206ms").is_none());
    }

    #[test]
    fn 相機幀格式() {
        assert_eq!(camera_frame("NoRead"), "@");
        assert_eq!(camera_frame("99K00064643"), "99K00064643;(0,0)(0,0)(0,0)(0,0);Code128@");
    }
}
