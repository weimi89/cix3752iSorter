//! 讀碼站照片：本程式當 FTP 伺服器，收讀碼器（海康 ID6200M）每件上傳的圖，當「這件我們有收到」的證據。
//!
//! 只做讀碼器會用到的那一小段 FTP：登入、PASV／PORT 兩種資料連線、`STOR` 上傳；
//! 下載、刪檔、列目錄一律拒絕（回空清單），它不是給人用的檔案伺服器。
//! 收到原圖後立刻縮成證據圖（`camera_ftp.max_edge_px`）存進 `data/images/YYYY-MM-DD/`，
//! 原圖不留（20MP 一張 2–3 MB，一天五千多件會把硬碟吃光）；讀碼失敗的件可另外保留原圖。
//!
//! 照片對回包裹：檔名裡若含窗口內某件的條碼就直接對上；否則取窗口內最早、還沒有照片的那件
//! （讀碼器照拍照順序上傳，先到的圖屬於先綁條碼的件）。對不到的照片仍存檔，`parcel_id` 留空。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::config::{AppConfig, CameraFtpConfig};
use crate::db::DbPool;
use crate::event_log::{self, Level};

/// 單張上傳上限；20MP JPEG 最多幾 MB，超過就是設定錯（BMP／RAW）或壞掉的連線
const MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;
/// 控制連線多久沒指令就踢掉
const CONTROL_IDLE: Duration = Duration::from_secs(300);
/// 資料連線建立與傳輸中的閒置上限
const DATA_TIMEOUT: Duration = Duration::from_secs(60);

/// 照片存放目錄：設定填了絕對路徑就用它，否則資料目錄下的 `images`
pub fn images_dir(cfg: &CameraFtpConfig, data_dir: &Path) -> PathBuf {
    let custom = cfg.images_dir.trim();
    if custom.is_empty() { data_dir.join("images") } else { PathBuf::from(custom) }
}

pub fn spawn(db: DbPool, data_dir: PathBuf, cfg_rx: watch::Receiver<AppConfig>, cancel: CancellationToken) {
    let ftp_rx = super::derive(cfg_rx, |c| c.camera_ftp.clone(), cancel.clone());
    tokio::spawn(run(db, data_dir, ftp_rx, cancel));
}

async fn run(db: DbPool, data_dir: PathBuf, mut cfg_rx: watch::Receiver<CameraFtpConfig>, cancel: CancellationToken) {
    loop {
        if cancel.is_cancelled() {
            return;
        }
        let cfg = cfg_rx.borrow_and_update().clone();
        if !cfg.enabled {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = cfg_rx.changed() => {}
            }
            continue;
        }
        let listener = match TcpListener::bind(&cfg.listen).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(addr = %cfg.listen, "讀碼站照片 FTP 埠開啟失敗: {e}");
                event_log::log(&db, Level::Error, "camera", "ftp", format!("讀碼站照片 FTP 埠 {} 開啟失敗：{e}", cfg.listen));
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                    _ = cfg_rx.changed() => {}
                }
                continue;
            }
        };
        tracing::info!(addr = %cfg.listen, "讀碼站照片 FTP 監聽啟動");
        let conn_cancel = cancel.child_token();
        loop {
            tokio::select! {
                _ = cancel.cancelled() => { conn_cancel.cancel(); return; }
                _ = cfg_rx.changed() => { tracing::info!("讀碼站照片 FTP 設定變更，重開"); conn_cancel.cancel(); break; }
                r = listener.accept() => match r {
                    Ok((stream, peer)) => {
                        let cfg = cfg_rx.borrow().clone();
                        let session = Session {
                            db: db.clone(),
                            images_dir: images_dir(&cfg, &data_dir),
                            cfg,
                            peer,
                            authed: false,
                            user: String::new(),
                            cwd: "/".into(),
                            data: DataMode::None,
                        };
                        tokio::spawn(session.serve(stream, conn_cancel.clone()));
                    }
                    Err(e) => {
                        tracing::warn!("讀碼站照片 FTP accept 失敗: {e}");
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                },
            }
        }
    }
}

enum DataMode {
    None,
    Passive(TcpListener),
    Active(SocketAddr),
}

struct Session {
    db: DbPool,
    images_dir: PathBuf,
    cfg: CameraFtpConfig,
    peer: SocketAddr,
    authed: bool,
    user: String,
    cwd: String,
    data: DataMode,
}

impl Session {
    async fn serve(mut self, stream: TcpStream, cancel: CancellationToken) {
        let peer = self.peer;
        tracing::info!(%peer, "讀碼站照片 FTP 連線");
        let (rd, mut wr) = stream.into_split();
        let mut lines = BufReader::new(rd).lines();
        if wr.write_all(b"220 cix3752i-sorter image drop ready\r\n").await.is_err() {
            return;
        }
        loop {
            let line = tokio::select! {
                _ = cancel.cancelled() => break,
                r = tokio::time::timeout(CONTROL_IDLE, lines.next_line()) => match r {
                    Ok(Ok(Some(l))) => l,
                    Ok(Ok(None)) => break,
                    Ok(Err(e)) => { tracing::debug!(%peer, "FTP 控制連線讀取錯誤: {e}"); break; }
                    Err(_) => { let _ = wr.write_all(b"421 Idle timeout\r\n").await; break; }
                },
            };
            let (cmd, arg) = split_cmd(&line);
            let reply = match self.handle(&cmd, arg, &mut wr).await {
                Step::Reply(r) => r,
                Step::Quit => {
                    let _ = wr.write_all(b"221 Bye\r\n").await;
                    break;
                }
            };
            if wr.write_all(reply.as_bytes()).await.is_err() {
                break;
            }
        }
        tracing::info!(%peer, "讀碼站照片 FTP 連線結束");
    }

    async fn handle(&mut self, cmd: &str, arg: &str, wr: &mut tokio::net::tcp::OwnedWriteHalf) -> Step {
        // 登入前只認這幾個
        if !self.authed {
            return match cmd {
                "USER" => {
                    self.user = arg.to_string();
                    Step::Reply("331 Password required\r\n".into())
                }
                "PASS" => {
                    if self.user == self.cfg.username && arg == self.cfg.password {
                        self.authed = true;
                        Step::Reply("230 Logged in\r\n".into())
                    } else {
                        // 拖慢一點，讓猜密碼沒有效率
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        tracing::warn!(peer = %self.peer, user = %self.user, "讀碼站照片 FTP 登入失敗");
                        Step::Reply("530 Login incorrect\r\n".into())
                    }
                }
                "QUIT" => Step::Quit,
                "FEAT" => Step::Reply(feat()),
                "SYST" => Step::Reply("215 UNIX Type: L8\r\n".into()),
                "AUTH" => Step::Reply("502 TLS not supported\r\n".into()),
                _ => Step::Reply("530 Please login\r\n".into()),
            };
        }
        match cmd {
            "QUIT" => Step::Quit,
            "NOOP" => Step::Reply("200 OK\r\n".into()),
            "SYST" => Step::Reply("215 UNIX Type: L8\r\n".into()),
            "FEAT" => Step::Reply(feat()),
            "OPTS" => Step::Reply("200 OK\r\n".into()),
            "TYPE" => Step::Reply("200 Type set\r\n".into()),
            "MODE" | "STRU" => Step::Reply("200 OK\r\n".into()),
            "PWD" | "XPWD" => Step::Reply(format!("257 \"{}\"\r\n", self.cwd)),
            "CWD" | "XCWD" => {
                self.cwd = virtual_join(&self.cwd, arg);
                Step::Reply("250 OK\r\n".into())
            }
            "CDUP" | "XCUP" => {
                self.cwd = virtual_join(&self.cwd, "..");
                Step::Reply("250 OK\r\n".into())
            }
            // 讀碼器可能想先建日期目錄；目錄只是虛的，存哪裡由本程式決定
            "MKD" | "XMKD" => Step::Reply(format!("257 \"{}\" created\r\n", virtual_join(&self.cwd, arg))),
            "PASV" => match self.open_passive().await {
                Ok(port) => {
                    let ip = match self.local_ip(wr) {
                        IpAddr::V4(v4) => v4,
                        IpAddr::V6(_) => Ipv4Addr::UNSPECIFIED,
                    };
                    let [a, b, c, d] = ip.octets();
                    Step::Reply(format!("227 Entering Passive Mode ({a},{b},{c},{d},{},{})\r\n", port >> 8, port & 0xff))
                }
                Err(e) => Step::Reply(format!("425 Cannot open data port: {e}\r\n")),
            },
            "EPSV" => match self.open_passive().await {
                Ok(port) => Step::Reply(format!("229 Entering Extended Passive Mode (|||{port}|)\r\n")),
                Err(e) => Step::Reply(format!("425 Cannot open data port: {e}\r\n")),
            },
            "PORT" => match parse_port(arg) {
                Some(addr) => {
                    self.data = DataMode::Active(addr);
                    Step::Reply("200 PORT OK\r\n".into())
                }
                None => Step::Reply("501 Bad PORT\r\n".into()),
            },
            "EPRT" => match parse_eprt(arg) {
                Some(addr) => {
                    self.data = DataMode::Active(addr);
                    Step::Reply("200 EPRT OK\r\n".into())
                }
                None => Step::Reply("501 Bad EPRT\r\n".into()),
            },
            "STOR" | "APPE" => self.store(arg, wr).await,
            "LIST" | "NLST" | "MLSD" => {
                // 回空清單就好；有些客戶端上傳前會先列一次
                match self.open_data().await {
                    Ok(mut data) => {
                        let _ = wr.write_all(b"150 Here comes the directory listing\r\n").await;
                        let _ = data.shutdown().await;
                        Step::Reply("226 Directory send OK\r\n".into())
                    }
                    Err(e) => Step::Reply(format!("425 {e}\r\n")),
                }
            }
            "RETR" | "DELE" | "RMD" | "XRMD" | "RNFR" | "RNTO" | "SIZE" | "MDTM" => Step::Reply("550 Not permitted\r\n".into()),
            "USER" | "PASS" => Step::Reply("230 Already logged in\r\n".into()),
            _ => Step::Reply("502 Command not implemented\r\n".into()),
        }
    }

    fn local_ip(&self, wr: &tokio::net::tcp::OwnedWriteHalf) -> IpAddr {
        wr.local_addr().map(|a| a.ip()).unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
    }

    /// 被動模式：在設定的埠範圍內找一個沒被占用的（防火牆只放這一段）；範圍是 0–0 就交給系統挑
    async fn open_passive(&mut self) -> std::io::Result<u16> {
        let (lo, hi) = (self.cfg.passive_port_min, self.cfg.passive_port_max);
        let l = if lo == 0 && hi == 0 {
            TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).await?
        } else {
            let (lo, hi) = (lo.min(hi).max(1), lo.max(hi));
            // 從隨機位置開始繞一圈，兩條連線同時談 PASV 時不會擠在同一個埠
            let span = (hi - lo) as u32 + 1;
            let start = (crate::db::now_ms() as u32) % span;
            let mut found = None;
            for i in 0..span {
                let port = lo + ((start + i) % span) as u16;
                if let Ok(l) = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await {
                    found = Some(l);
                    break;
                }
            }
            found.ok_or_else(|| std::io::Error::other(format!("passive ports {lo}-{hi} all busy")))?
        };
        let port = l.local_addr()?.port();
        self.data = DataMode::Passive(l);
        Ok(port)
    }

    /// 依上一個 PASV／PORT 建立資料連線；用過一次就作廢（FTP 每次傳輸都要重新談）
    async fn open_data(&mut self) -> Result<TcpStream, String> {
        match std::mem::replace(&mut self.data, DataMode::None) {
            DataMode::None => Err("Use PASV or PORT first".into()),
            DataMode::Passive(l) => match tokio::time::timeout(DATA_TIMEOUT, l.accept()).await {
                Ok(Ok((s, _))) => Ok(s),
                Ok(Err(e)) => Err(format!("data accept failed: {e}")),
                Err(_) => Err("data connection timeout".into()),
            },
            DataMode::Active(addr) => match tokio::time::timeout(DATA_TIMEOUT, TcpStream::connect(addr)).await {
                Ok(Ok(s)) => Ok(s),
                Ok(Err(e)) => Err(format!("data connect failed: {e}")),
                Err(_) => Err("data connect timeout".into()),
            },
        }
    }

    async fn store(&mut self, name: &str, wr: &mut tokio::net::tcp::OwnedWriteHalf) -> Step {
        let file_name = sanitize_name(name);
        if file_name.is_empty() {
            return Step::Reply("553 Bad file name\r\n".into());
        }
        let mut data = match self.open_data().await {
            Ok(d) => d,
            Err(e) => return Step::Reply(format!("425 {e}\r\n")),
        };
        if wr.write_all(b"150 Ok to send data\r\n").await.is_err() {
            return Step::Quit;
        }
        let mut bytes = Vec::with_capacity(1 << 20);
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            match tokio::time::timeout(DATA_TIMEOUT, data.read(&mut chunk)).await {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => {
                    bytes.extend_from_slice(&chunk[..n]);
                    if bytes.len() > MAX_UPLOAD_BYTES {
                        return Step::Reply("552 File too large\r\n".into());
                    }
                }
                Ok(Err(e)) => return Step::Reply(format!("426 Transfer aborted: {e}\r\n")),
                Err(_) => return Step::Reply("426 Transfer timeout\r\n".into()),
            }
        }
        let received_ms = crate::db::now_ms();
        match ingest(&self.db, &self.images_dir, &self.cfg, &file_name, bytes, received_ms).await {
            Ok(()) => Step::Reply("226 Transfer complete\r\n".into()),
            Err(e) => {
                tracing::error!(file = %file_name, "讀碼站照片存檔失敗: {e:#}");
                event_log::log(&self.db, Level::Error, "camera", "image", format!("照片 {file_name} 存檔失敗：{e:#}"));
                Step::Reply("451 Local error\r\n".into())
            }
        }
    }
}

enum Step {
    Reply(String),
    Quit,
}

fn feat() -> String {
    "211-Features:\r\n UTF8\r\n EPSV\r\n EPRT\r\n211 End\r\n".into()
}

fn split_cmd(line: &str) -> (String, &str) {
    let line = line.trim_end_matches(['\r', '\n']);
    match line.split_once(' ') {
        Some((c, a)) => (c.trim().to_ascii_uppercase(), a.trim()),
        None => (line.trim().to_ascii_uppercase(), ""),
    }
}

/// 虛擬目錄：只為了 PWD 回得像樣，跟實際存放位置無關
fn virtual_join(cwd: &str, arg: &str) -> String {
    let mut parts: Vec<&str> = if arg.starts_with('/') { Vec::new() } else { cwd.split('/').filter(|s| !s.is_empty()).collect() };
    for seg in arg.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}

/// 檔名只留最後一段、只准安全字元；副檔名統一小寫
pub fn sanitize_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let cleaned: String = base.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')).take(120).collect();
    let cleaned = cleaned.trim_matches('.').to_string();
    match cleaned.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}.{}", ext.to_ascii_lowercase()),
        _ => cleaned,
    }
}

fn parse_port(arg: &str) -> Option<SocketAddr> {
    let n: Vec<u16> = arg.split(',').map(|s| s.trim().parse::<u16>().ok()).collect::<Option<Vec<_>>>()?;
    if n.len() != 6 || n[..4].iter().any(|&v| v > 255) || n[4] > 255 || n[5] > 255 {
        return None;
    }
    let ip = Ipv4Addr::new(n[0] as u8, n[1] as u8, n[2] as u8, n[3] as u8);
    Some(SocketAddr::new(IpAddr::V4(ip), n[4] * 256 + n[5]))
}

/// `|1|192.168.177.20|50000|`
fn parse_eprt(arg: &str) -> Option<SocketAddr> {
    let delim = arg.chars().next()?;
    let parts: Vec<&str> = arg.split(delim).collect();
    if parts.len() < 5 {
        return None;
    }
    let ip: IpAddr = parts[2].parse().ok()?;
    let port: u16 = parts[3].parse().ok()?;
    Some(SocketAddr::new(ip, port))
}

/// 存證據圖（必要時保留原圖）、寫 `parcel_images`、對回包裹
async fn ingest(db: &DbPool, images_dir: &Path, cfg: &CameraFtpConfig, file_name: &str, bytes: Vec<u8>, received_ms: i64) -> anyhow::Result<()> {
    let orig_size = bytes.len() as i64;
    let day = crate::db::local_ts(received_ms)[..10].to_string();
    let day_dir = images_dir.join(&day);
    tokio::fs::create_dir_all(&day_dir).await?;

    let max_edge = cfg.max_edge_px;
    let quality = cfg.jpeg_quality.clamp(1, 100);
    let (evidence, orig_bytes) = tokio::task::spawn_blocking({
        let bytes = bytes;
        move || -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
            let evidence = shrink(&bytes, max_edge, quality)?;
            Ok((evidence, bytes))
        }
    })
    .await??;

    let stem = file_name.rsplit_once('.').map(|(s, _)| s).unwrap_or(file_name);
    let evidence_name = unique_name(&day_dir, stem, "jpg").await;
    let evidence_path = day_dir.join(&evidence_name);
    crate::fs_atomic::write_async(&evidence_path, &evidence).await?;
    let rel_path = format!("{day}/{evidence_name}");

    let parcel = match_parcel(db, cfg, file_name, received_ms).await?;

    // 讀碼失敗的件另留原圖：縮圖看不出為什麼讀不到
    let mut orig_rel: Option<String> = None;
    if cfg.keep_original_noread && parcel.as_ref().is_some_and(|(_, bc)| bc == super::camera::NO_READ) && max_edge > 0 {
        let orig_dir = day_dir.join("orig");
        tokio::fs::create_dir_all(&orig_dir).await?;
        let ext = file_name.rsplit_once('.').map(|(_, e)| e).unwrap_or("jpg");
        let orig_name = unique_name(&orig_dir, stem, ext).await;
        crate::fs_atomic::write_async(&orig_dir.join(&orig_name), &orig_bytes).await?;
        orig_rel = Some(format!("{day}/orig/{orig_name}"));
    }

    let parcel_id = parcel.as_ref().map(|(id, _)| *id);
    let image_id: i64 = sqlx::query_scalar(
        "INSERT INTO parcel_images (parcel_id, file_name, rel_path, orig_path, size, orig_size, received_ms, received_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(parcel_id)
    .bind(file_name)
    .bind(&rel_path)
    .bind(&orig_rel)
    .bind(evidence.len() as i64)
    .bind(orig_size)
    .bind(received_ms)
    .bind(crate::db::local_ts(received_ms))
    .fetch_one(db)
    .await?;

    match &parcel {
        Some((id, barcode)) => {
            sqlx::query("INSERT INTO parcel_events (parcel_id, ts_ms, source, kind, raw) VALUES (?, ?, 'camera', 'image', ?)")
                .bind(id)
                .bind(received_ms)
                .bind(format!("{file_name} ({} KB)", evidence.len() / 1024))
                .execute(db)
                .await?;
            tracing::debug!(file = %file_name, parcel = id, %barcode, "讀碼站照片已對到包裹");
        }
        None => {
            // 對不到才記事件：正常一天幾千張，逐張記會淹掉事件記錄
            event_log::log(db, Level::Warn, "camera", "image", format!("照片 {file_name} 對不到包裹（{} 秒內沒有剛綁條碼的件），已存檔", cfg.match_window_ms / 1000));
        }
    }
    crate::event_bus::emit("parcel-image", serde_json::json!({ "image_id": image_id, "parcel_id": parcel_id, "file_name": file_name }));
    Ok(())
}

/// 縮成證據圖：長邊不超過 `max_edge`（0 = 原樣回傳），JPEG 品質 `quality`
pub fn shrink(bytes: &[u8], max_edge: u32, quality: u8) -> anyhow::Result<Vec<u8>> {
    if max_edge == 0 {
        return Ok(bytes.to_vec());
    }
    let img = image::load_from_memory(bytes)?;
    let (w, h) = (img.width(), img.height());
    let img = if w.max(h) > max_edge {
        // Triangle 比 Lanczos 快好幾倍，證據圖看得清面單就夠
        img.resize(max_edge, max_edge, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut out = Vec::with_capacity(256 * 1024);
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    // 黑貓站的讀碼器是單色機；彩色機也照原色深存
    match img {
        image::DynamicImage::ImageLuma8(g) => enc.encode_image(&g)?,
        other => enc.encode_image(&other.to_rgb8())?,
    }
    Ok(out)
}

/// 同名檔已存在就補流水號，不覆蓋
async fn unique_name(dir: &Path, stem: &str, ext: &str) -> String {
    let stem = if stem.is_empty() { "image" } else { stem };
    let mut name = format!("{stem}.{ext}");
    let mut n = 1;
    while tokio::fs::try_exists(dir.join(&name)).await.unwrap_or(false) {
        name = format!("{stem}-{n}.{ext}");
        n += 1;
    }
    name
}

/// 照片對回包裹：窗口內剛綁條碼、還沒有照片的件。檔名含其中某件的條碼就對那件；
/// 否則取最早的那件（讀碼器照拍照順序上傳）。
pub async fn match_parcel(db: &DbPool, cfg: &CameraFtpConfig, file_name: &str, received_ms: i64) -> Result<Option<(i64, String)>, sqlx::Error> {
    let since = received_ms - cfg.match_window_ms.max(0);
    let candidates: Vec<(i64, String)> = sqlx::query_as(
        "SELECT p.id, p.barcode
           FROM parcels p
           JOIN parcel_events e ON e.parcel_id = p.id AND e.source = 'camera' AND e.kind = 'bind'
          WHERE e.ts_ms BETWEEN ? AND ?
            AND NOT EXISTS (SELECT 1 FROM parcel_images i WHERE i.parcel_id = p.id)
          ORDER BY e.ts_ms ASC",
    )
    .bind(since)
    .bind(received_ms + 500)
    .fetch_all(db)
    .await?;
    if let Some(hit) = candidates.iter().find(|(_, bc)| bc != super::camera::NO_READ && bc.len() >= 6 && file_name.contains(bc.as_str())) {
        return Ok(Some(hit.clone()));
    }
    Ok(candidates.into_iter().next())
}

/// 清掉 `days` 天前的照片（檔案與資料列），順手移除空掉的日期目錄
pub async fn purge(db: &DbPool, images_dir: &Path, days: u32) -> Result<u64, sqlx::Error> {
    if days == 0 {
        return Ok(0);
    }
    let cutoff_day = (chrono::Local::now() - chrono::Duration::days(days as i64)).format("%Y-%m-%d").to_string();
    let cutoff_ts = format!("{cutoff_day} 00:00:00");
    let rows: Vec<(i64, String, Option<String>)> = sqlx::query_as("SELECT id, rel_path, orig_path FROM parcel_images WHERE received_at < ?").bind(&cutoff_ts).fetch_all(db).await?;
    for (_, rel, orig) in &rows {
        let _ = tokio::fs::remove_file(images_dir.join(rel)).await;
        if let Some(o) = orig {
            let _ = tokio::fs::remove_file(images_dir.join(o)).await;
        }
    }
    let n = sqlx::query("DELETE FROM parcel_images WHERE received_at < ?").bind(&cutoff_ts).execute(db).await?.rows_affected();
    // 日期目錄名就是 YYYY-MM-DD，早於截止日且已空的一併拿掉
    if let Ok(mut rd) = tokio::fs::read_dir(images_dir).await {
        while let Ok(Some(ent)) = rd.next_entry().await {
            let name = ent.file_name().to_string_lossy().to_string();
            if name.len() == 10 && name.as_str() < cutoff_day.as_str() {
                let _ = tokio::fs::remove_dir(ent.path().join("orig")).await;
                let _ = tokio::fs::remove_dir(ent.path()).await;
            }
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 檔名只留安全字元與最後一段() {
        assert_eq!(sanitize_name("/2026-09-21/20260921_151130123_5_1234.JPG"), "20260921_151130123_5_1234.jpg");
        assert_eq!(sanitize_name("..\\..\\etc\\passwd"), "passwd");
        assert_eq!(sanitize_name("a b;c$.jpeg"), "abc.jpeg");
        assert_eq!(sanitize_name("...."), "");
    }

    #[test]
    fn 虛擬目錄() {
        assert_eq!(virtual_join("/", "2026-09-21"), "/2026-09-21");
        assert_eq!(virtual_join("/a/b", ".."), "/a");
        assert_eq!(virtual_join("/a/b", "/x"), "/x");
        assert_eq!(virtual_join("/", ".."), "/");
    }

    #[test]
    fn port_與_eprt_解析() {
        assert_eq!(parse_port("192,168,177,20,195,80"), Some("192.168.177.20:50000".parse().unwrap()));
        assert_eq!(parse_port("1,2,3"), None);
        assert_eq!(parse_port("300,1,1,1,1,1"), None);
        assert_eq!(parse_eprt("|1|192.168.177.20|50001|"), Some("192.168.177.20:50001".parse().unwrap()));
        assert_eq!(parse_eprt("garbage"), None);
    }

    fn test_jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::GrayImage::from_fn(w, h, |x, y| image::Luma([((x + y) % 256) as u8]));
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90).encode_image(&img).unwrap();
        out
    }

    #[test]
    fn 縮圖_長邊到上限_小圖不放大_零為原樣() {
        let big = test_jpeg(4000, 2000);
        let small = shrink(&big, 1600, 80).unwrap();
        let dims = image::load_from_memory(&small).unwrap();
        assert_eq!((dims.width(), dims.height()), (1600, 800));
        assert!(small.len() < big.len());

        let tiny = test_jpeg(300, 200);
        let same = image::load_from_memory(&shrink(&tiny, 1600, 80).unwrap()).unwrap();
        assert_eq!((same.width(), same.height()), (300, 200));

        assert_eq!(shrink(&tiny, 0, 80).unwrap(), tiny, "0 = 原樣回傳");
        assert!(shrink(b"not an image", 1600, 80).is_err());
    }

    async fn test_db() -> (DbPool, PathBuf) {
        let dir = std::env::temp_dir().join(format!("camera-ftp-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        (crate::db::init(&dir).await.unwrap(), dir)
    }

    async fn seed_parcel(db: &DbPool, barcode: &str, bind_ms: i64) -> i64 {
        let started_at = crate::db::local_ts(bind_ms - 200);
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO parcels (ulid, barcode, status, started_at, started_ms, updated_ms) VALUES (?, ?, 2, ?, ?, ?) RETURNING id",
        )
        .bind(ulid::Ulid::generate().to_string())
        .bind(barcode)
        .bind(&started_at)
        .bind(bind_ms - 200)
        .bind(bind_ms)
        .fetch_one(db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO parcel_events (parcel_id, ts_ms, source, kind, raw) VALUES (?, ?, 'camera', 'bind', ?)")
            .bind(id)
            .bind(bind_ms)
            .bind(barcode)
            .execute(db)
            .await
            .unwrap();
        id
    }

    #[tokio::test]
    async fn 對包裹_檔名含條碼優先_否則取最早未配對的_窗口外不算() {
        let (db, _dir) = test_db().await;
        let cfg = CameraFtpConfig::default();
        let now = crate::db::now_ms();
        let a = seed_parcel(&db, "SF1234567890123", now - 3000).await;
        let b = seed_parcel(&db, "NoRead", now - 2000).await;
        let _old = seed_parcel(&db, "NoRead", now - 20_000).await;

        // 檔名帶條碼 → 直接對 a，即使 b 也在窗口內
        let hit = match_parcel(&db, &cfg, "SF1234567890123_20260921.jpg", now).await.unwrap();
        assert_eq!(hit.map(|(id, _)| id), Some(a));
        // 檔名沒條碼 → 最早的那件（a 還沒有照片）
        let hit = match_parcel(&db, &cfg, "20260921_151130123_5_1.jpg", now).await.unwrap();
        assert_eq!(hit.map(|(id, _)| id), Some(a));
        // a 有照片後 → 換 b
        sqlx::query("INSERT INTO parcel_images (parcel_id, file_name, rel_path, size, orig_size, received_ms, received_at) VALUES (?, 'x.jpg', 'd/x.jpg', 1, 1, ?, '2026-09-21 00:00:00.000')")
            .bind(a)
            .bind(now)
            .execute(&db)
            .await
            .unwrap();
        let hit = match_parcel(&db, &cfg, "20260921_151130123_5_2.jpg", now).await.unwrap();
        assert_eq!(hit.map(|(id, bc)| (id, bc)), Some((b, "NoRead".to_string())));
        // b 也有了 → 20 秒前那件在窗口外，對不到
        sqlx::query("INSERT INTO parcel_images (parcel_id, file_name, rel_path, size, orig_size, received_ms, received_at) VALUES (?, 'y.jpg', 'd/y.jpg', 1, 1, ?, '2026-09-21 00:00:00.000')")
            .bind(b)
            .bind(now)
            .execute(&db)
            .await
            .unwrap();
        assert!(match_parcel(&db, &cfg, "20260921_151130123_5_3.jpg", now).await.unwrap().is_none());
    }

    /// 讀到指定代碼的那一行回覆（多行回覆 `211-` 跳到結尾那行）
    async fn expect(lines: &mut tokio::io::Lines<BufReader<tokio::net::tcp::OwnedReadHalf>>, code: &str) -> String {
        loop {
            let l = lines.next_line().await.unwrap().expect("伺服器提早關閉");
            if l.starts_with(code) && l.chars().nth(3) == Some(' ') {
                return l;
            }
            assert!(l.starts_with(code) || l.starts_with(' '), "預期 {code}，得到 {l}");
        }
    }

    /// 走完整 FTP：登入 → PASV 上傳一張 → PORT 上傳一張；證據圖落地、資料列與事件都在
    #[tokio::test]
    async fn ftp_上傳_pasv_與_port_兩種模式() {
        let (db, dir) = test_db().await;
        let images_dir = dir.join("images");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let cfg = CameraFtpConfig { keep_original_noread: true, ..CameraFtpConfig::default() };
        let cancel = CancellationToken::new();
        {
            let (db, images_dir, cfg, cancel) = (db.clone(), images_dir.clone(), cfg.clone(), cancel.clone());
            tokio::spawn(async move {
                loop {
                    let (stream, peer) = listener.accept().await.unwrap();
                    let s = Session { db: db.clone(), images_dir: images_dir.clone(), cfg: cfg.clone(), peer, authed: false, user: String::new(), cwd: "/".into(), data: DataMode::None };
                    tokio::spawn(s.serve(stream, cancel.clone()));
                }
            });
        }
        let now = crate::db::now_ms();
        let p_noread = seed_parcel(&db, "NoRead", now - 1500).await;
        let p_ok = seed_parcel(&db, "SF9876543210123", now - 1000).await;

        let ctl = TcpStream::connect(addr).await.unwrap();
        let (rd, mut wr) = ctl.into_split();
        let mut lines = BufReader::new(rd).lines();
        expect(&mut lines, "220").await;
        wr.write_all(b"USER sorter\r\n").await.unwrap();
        expect(&mut lines, "331").await;
        wr.write_all(b"PASS wrong\r\n").await.unwrap();
        expect(&mut lines, "530").await;
        wr.write_all(b"USER sorter\r\nPASS sorter\r\n").await.unwrap();
        expect(&mut lines, "331").await;
        expect(&mut lines, "230").await;
        wr.write_all(b"TYPE I\r\n").await.unwrap();
        expect(&mut lines, "200").await;
        wr.write_all(b"CWD 2026-09-21\r\n").await.unwrap();
        expect(&mut lines, "250").await;

        // PASV：第一張，對到讀碼失敗那件 → 有原圖
        wr.write_all(b"PASV\r\n").await.unwrap();
        let l = expect(&mut lines, "227").await;
        let nums: Vec<u16> = l[l.find('(').unwrap() + 1..l.find(')').unwrap()].split(',').map(|s| s.parse().unwrap()).collect();
        let data_port = nums[4] * 256 + nums[5];
        wr.write_all(b"STOR 20260921_151130123_5_1.jpg\r\n").await.unwrap();
        let mut data = TcpStream::connect(("127.0.0.1", data_port)).await.unwrap();
        expect(&mut lines, "150").await;
        data.write_all(&test_jpeg(2400, 1800)).await.unwrap();
        data.shutdown().await.unwrap();
        drop(data);
        expect(&mut lines, "226").await;

        // PORT：第二張，對到讀取成功那件（檔名帶條碼）
        let dl = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dp = dl.local_addr().unwrap().port();
        wr.write_all(format!("PORT 127,0,0,1,{},{}\r\n", dp >> 8, dp & 0xff).as_bytes()).await.unwrap();
        expect(&mut lines, "200").await;
        wr.write_all(b"STOR SF9876543210123_20260921_151131000.jpg\r\n").await.unwrap();
        let (mut data, _) = dl.accept().await.unwrap();
        expect(&mut lines, "150").await;
        data.write_all(&test_jpeg(800, 600)).await.unwrap();
        data.shutdown().await.unwrap();
        drop(data);
        expect(&mut lines, "226").await;

        wr.write_all(b"RETR x\r\n").await.unwrap();
        expect(&mut lines, "550").await;
        wr.write_all(b"QUIT\r\n").await.unwrap();
        expect(&mut lines, "221").await;

        let rows: Vec<(Option<i64>, String, String, Option<String>, i64)> =
            sqlx::query_as("SELECT parcel_id, file_name, rel_path, orig_path, size FROM parcel_images ORDER BY id").fetch_all(&db).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, Some(p_noread));
        assert!(rows[0].3.is_some(), "讀碼失敗件要留原圖");
        assert!(images_dir.join(&rows[0].2).is_file());
        assert!(images_dir.join(rows[0].3.as_ref().unwrap()).is_file());
        let shrunk = image::load_from_memory(&std::fs::read(images_dir.join(&rows[0].2)).unwrap()).unwrap();
        assert_eq!((shrunk.width(), shrunk.height()), (1600, 1200));
        assert_eq!(rows[1].0, Some(p_ok));
        assert!(rows[1].3.is_none(), "讀得到的件不留原圖");
        let ev: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM parcel_events WHERE kind = 'image'").fetch_one(&db).await.unwrap();
        assert_eq!(ev, 2);
        cancel.cancel();
    }

    #[tokio::test]
    async fn 清理_過期照片連檔案一起刪_目錄清空() {
        let (db, dir) = test_db().await;
        let images_dir = dir.join("images");
        let old_day = (chrono::Local::now() - chrono::Duration::days(100)).format("%Y-%m-%d").to_string();
        let new_day = chrono::Local::now().format("%Y-%m-%d").to_string();
        for (day, name) in [(&old_day, "a.jpg"), (&new_day, "b.jpg")] {
            std::fs::create_dir_all(images_dir.join(day).join("orig")).unwrap();
            std::fs::write(images_dir.join(day).join(name), b"x").unwrap();
            std::fs::write(images_dir.join(day).join("orig").join(name), b"x").unwrap();
            sqlx::query("INSERT INTO parcel_images (file_name, rel_path, orig_path, size, orig_size, received_ms, received_at) VALUES (?, ?, ?, 1, 1, 0, ?)")
                .bind(name)
                .bind(format!("{day}/{name}"))
                .bind(format!("{day}/orig/{name}"))
                .bind(format!("{day} 12:00:00.000"))
                .execute(&db)
                .await
                .unwrap();
        }
        assert_eq!(purge(&db, &images_dir, 90).await.unwrap(), 1);
        assert!(!images_dir.join(&old_day).exists(), "舊日期目錄整個拿掉");
        assert!(images_dir.join(&new_day).join("b.jpg").is_file());
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM parcel_images").fetch_one(&db).await.unwrap();
        assert_eq!(left, 1);
        assert_eq!(purge(&db, &images_dir, 0).await.unwrap(), 0, "0 = 不清");
    }
}
