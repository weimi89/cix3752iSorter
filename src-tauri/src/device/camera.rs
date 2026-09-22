//! 讀碼站相機：本程式當 TCP server，相機連進來送幀。
//!
//! 幀格式：`code;coord;type;code;coord;type;…@`（每三段一組，`@` 結尾）。
//! 一幀可能含多個條碼（面單上的一維碼、QR、內部序號…），`pick_barcode` 依
//! 舊 `server.js checkcode1` 的規則挑出物流單號；挑不到回 `NoRead`。
//!
//! 相機視野涵蓋到進料區，下一件的 QR 常在畫面上緣就被讀到，同一幀會有兩個合法 QR；
//! 這時取離畫面中心最近的那個，貼邊的是鄰件。挑錯會把鄰件的單號綁到這件：這件分到別人的口、
//! 鄰件之後又被分一次（9/21 一天 17 件）。

use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use super::events::{Device, DeviceEvent};
use super::signal_log::{self, Dir};
use crate::config::AppConfig;

pub const NO_READ: &str = "NoRead";

#[derive(Clone, Debug, PartialEq)]
pub struct Barcode {
    pub code: String,
    pub len: usize,
    pub kind: String,
    /// 條碼四角座標的中心（畫面像素）；相機沒給座標時為 None
    pub center: Option<(f64, f64)>,
}

/// `(x,y)(x,y)(x,y)(x,y)` → 四角平均；格式不對回 None
fn parse_center(coords: &str) -> Option<(f64, f64)> {
    let mut n = 0.0;
    let (mut sx, mut sy) = (0.0, 0.0);
    for pair in coords.split(')').map(|s| s.trim_start_matches('(').trim()).filter(|s| !s.is_empty()) {
        let (x, y) = pair.split_once(',')?;
        sx += x.trim().parse::<f64>().ok()?;
        sy += y.trim().parse::<f64>().ok()?;
        n += 1.0;
    }
    (n > 0.0).then(|| (sx / n, sy / n))
}

/// 把一幀拆成條碼清單（每三段一組；尾段不足三段的忽略）
pub fn parse_frame(frame: &str) -> Vec<Barcode> {
    let frame = frame.trim().trim_end_matches('@');
    let parts: Vec<&str> = frame.split(';').collect();
    let mut out = Vec::new();
    for chunk in parts.chunks(3) {
        if chunk.len() < 3 {
            break;
        }
        let code = chunk[0].trim();
        if code.is_empty() {
            continue;
        }
        out.push(Barcode { code: code.to_string(), len: code.chars().count(), kind: chunk[2].trim().to_string(), center: parse_center(chunk[1]) });
    }
    out
}

fn is_sf(code: &str) -> bool {
    code.len() == 15 && code.starts_with("SF") && code[2..].bytes().all(|b| b.is_ascii_digit())
}

/// 一維條碼黑名單：現場包材／內部序號的固定區間，不是物流單號
fn is_blacklisted(code: &str) -> bool {
    let Ok(v) = code.parse::<u64>() else { return false };
    (5081794201..=5081794210).contains(&v)
        || (8341748001..=8341748020).contains(&v)
        || (8341748051..=8341748070).contains(&v)
        || v == 8341748099
        || (8914810401..=8914810420).contains(&v)
        || (8914810451..=8914810470).contains(&v)
        || v == 8914810499
        || (9683951001..=9683951010).contains(&v)
        || v == 9683951099
}

fn is_valid_qr(bc: &Barcode) -> bool {
    bc.kind == "QRCode" && !bc.code.contains('^') && bc.len > 9
}

/// 依優先序挑出物流單號（移植 `checkcode1`）：
/// 1. 順豐 `SF` + 13 碼最優先
/// 2. 合法 QRCode（無 `^` 且長度 >9）；同幀不只一個且知道畫面中心時，取離中心最近的（貼邊的是鄰件）
/// 3. 一維碼：去掉長度 <9 與黑名單；同時有 16 碼與 9 碼 → 拼成 `9碼+16碼`（超商）；否則取最短的第一個
/// 4. 都沒有 → `NoRead`
///
/// `frame_center` 是畫面中心像素座標；None 或條碼沒座標時退回字典序（舊行為）
pub fn pick_barcode(mut codes: Vec<Barcode>, frame_center: Option<(f64, f64)>) -> String {
    codes.sort_by(|a, b| a.len.cmp(&b.len).then_with(|| a.code.cmp(&b.code)));

    if let Some(sf) = codes.iter().find(|bc| is_sf(&bc.code)) {
        return sf.code.clone();
    }
    let qrs: Vec<&Barcode> = codes.iter().filter(|bc| is_valid_qr(bc)).collect();
    if let Some((cx, cy)) = frame_center.filter(|_| qrs.len() > 1) {
        let dist = |bc: &Barcode| bc.center.map(|(x, y)| (x - cx).powi(2) + (y - cy).powi(2));
        if let Some(best) = qrs.iter().filter(|bc| dist(bc).is_some()).min_by(|a, b| dist(a).partial_cmp(&dist(b)).unwrap_or(std::cmp::Ordering::Equal)) {
            return best.code.clone();
        }
    }
    if let Some(qr) = qrs.first() {
        return qr.code.clone();
    }

    let mut c16 = None;
    let mut c9 = None;
    let mut all = Vec::new();
    for bc in &codes {
        if bc.kind == "QRCode" || bc.len < 9 || is_blacklisted(&bc.code) {
            continue;
        }
        if bc.len == 16 {
            c16 = Some(bc.code.clone());
        }
        if bc.len == 9 && !bc.code.contains('^') {
            c9 = Some(bc.code.clone());
        }
        all.push(bc.code.clone());
    }
    if let (Some(a), Some(b)) = (&c9, &c16) {
        return format!("{a}{b}");
    }
    all.into_iter().next().unwrap_or_else(|| NO_READ.to_string())
}

/// 去重後這一幀要送給狀態機的條碼。同一個碼幾秒內又出現＝上一件的面單還在畫面邊緣、這一件自己的碼沒讀到，
/// 當作讀碼失敗送出（不是丟掉）：件才會立刻綁 NoRead、有綁碼事件讓照片對得上，而不是等綁碼窗口過了才被判成讀碼失敗、照片還對不到
pub fn frame_code(dedup: &mut Dedup, code: String, ts_ms: i64) -> String {
    if dedup.accept(&code, ts_ms) {
        code
    } else {
        tracing::info!(%code, "相機重複條碼，這一幀視為讀碼失敗");
        NO_READ.to_string()
    }
}

/// 畫面中心；任一邊為 0 代表不用座標挑碼
pub fn frame_center(width: u32, height: u32) -> Option<(f64, f64)> {
    (width > 0 && height > 0).then(|| (width as f64 / 2.0, height as f64 / 2.0))
}

/// 同一條碼短時間內不重複觸發（`NoRead` 例外，每一幀都算）
pub struct Dedup {
    last: String,
    last_ms: i64,
    window_ms: i64,
}

impl Dedup {
    fn accept(&mut self, code: &str, now_ms: i64) -> bool {
        if now_ms - self.last_ms > self.window_ms {
            self.last.clear();
        }
        if code == NO_READ || code != self.last {
            self.last = code.to_string();
            self.last_ms = now_ms;
            true
        } else {
            false
        }
    }
}

pub fn spawn(cfg_rx: watch::Receiver<AppConfig>, out: mpsc::Sender<DeviceEvent>, cancel: CancellationToken) {
    let listen_rx = super::derive_addr(cfg_rx.clone(), |c| c.camera.listen.clone(), cancel.clone());
    tokio::spawn(run(listen_rx, cfg_rx, out, cancel));
}

async fn run(
    mut listen_rx: watch::Receiver<String>,
    cfg_rx: watch::Receiver<AppConfig>,
    out: mpsc::Sender<DeviceEvent>,
    cancel: CancellationToken,
) {
    loop {
        if cancel.is_cancelled() {
            return;
        }
        let addr = listen_rx.borrow_and_update().clone();
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(%addr, "相機監聽埠開啟失敗: {e}");
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(3)) => {}
                    _ = listen_rx.changed() => {}
                }
                continue;
            }
        };
        tracing::info!(%addr, "相機監聽啟動");
        let conn_cancel = cancel.child_token();

        loop {
            tokio::select! {
                _ = cancel.cancelled() => { conn_cancel.cancel(); return; }
                _ = listen_rx.changed() => { tracing::info!("相機監聽位址變更，重開"); conn_cancel.cancel(); break; }
                r = listener.accept() => match r {
                    Ok((stream, peer)) => {
                        let peer = peer.to_string();
                        tracing::info!(%peer, "相機連線");
                        signal_log::record("camera", Dir::Info, &format!("connected {peer}"));
                        let _ = out.send(DeviceEvent::State { device: Device::Camera, connected: true, detail: peer.clone(), ts_ms: crate::db::now_ms() }).await;
                        tokio::spawn(serve_camera(stream, peer, cfg_rx.clone(), out.clone(), conn_cancel.clone()));
                    }
                    Err(e) => {
                        tracing::warn!("相機 accept 失敗: {e}");
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                },
            }
        }
    }
}

async fn serve_camera(
    mut stream: tokio::net::TcpStream,
    peer: String,
    cfg_rx: watch::Receiver<AppConfig>,
    out: mpsc::Sender<DeviceEvent>,
    cancel: CancellationToken,
) {
    let mut dedup = Dedup { last: String::new(), last_ms: 0, window_ms: cfg_rx.borrow().camera.dedup_ms as i64 };
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let reason;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => { reason = "服務關閉".to_string(); break; }
            r = stream.read(&mut chunk) => match r {
                Ok(0) => { reason = "相機端關閉".to_string(); break; }
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    // 一次 read 可能含多幀或半幀，以 `@` 切
                    while let Some(pos) = buf.iter().position(|&b| b == b'@') {
                        let frame_bytes: Vec<u8> = buf.drain(..=pos).collect();
                        let frame = String::from_utf8_lossy(&frame_bytes).into_owned();
                        let ts_ms = crate::db::now_ms();
                        let (code, window) = {
                            let c = cfg_rx.borrow();
                            (pick_barcode(parse_frame(&frame), frame_center(c.camera.frame_width, c.camera.frame_height)), c.camera.dedup_ms as i64)
                        };
                        dedup.window_ms = window;
                        let code = frame_code(&mut dedup, code, ts_ms);
                        signal_log::record("camera", Dir::In, &format!("{} => {code}", frame.trim()));
                        if out.send(DeviceEvent::Barcode { code, raw: frame.trim().to_string(), ts_ms }).await.is_err() {
                            return;
                        }
                    }
                    if buf.len() > 64 * 1024 {
                        tracing::warn!(%peer, "相機資料 64KB 內沒有幀結尾，清空緩衝");
                        buf.clear();
                    }
                }
                Err(e) => { reason = format!("讀取錯誤: {e}"); break; }
            },
        }
    }
    tracing::warn!(%peer, "相機連線結束: {reason}");
    signal_log::record("camera", Dir::Info, &format!("disconnected {peer}: {reason}"));
    let _ = out
        .send(DeviceEvent::State { device: Device::Camera, connected: false, detail: format!("{peer}: {reason}"), ts_ms: crate::db::now_ms() })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bc(code: &str, kind: &str) -> Barcode {
        Barcode { code: code.into(), len: code.chars().count(), kind: kind.into(), center: None }
    }

    fn qr_at(code: &str, x: f64, y: f64) -> Barcode {
        Barcode { code: code.into(), len: code.chars().count(), kind: "QRCode".into(), center: Some((x, y)) }
    }

    #[test]
    fn 幀拆解每三段一組() {
        let v = parse_frame("99K00064643;(1,2)(3,4)(5,6)(7,8);Code128;ABC;(0,0)(0,0)(0,0)(0,0);QRCode@");
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].code, "99K00064643");
        assert_eq!(v[0].center, Some((4.0, 5.0)));
        assert_eq!(v[1].kind, "QRCode");
        assert_eq!(parse_frame("X1234567890;;QRCode@")[0].center, None, "沒座標也要能解");
        assert!(parse_frame("@").is_empty());
        assert!(parse_frame("").is_empty());
    }

    #[test]
    fn 順豐最優先() {
        let code = pick_barcode(vec![bc("1234567890123456", "Code128"), bc("SF1234567890123", "Code128")], None);
        assert_eq!(code, "SF1234567890123");
    }

    #[test]
    fn 合法qr優先於一維() {
        let code = pick_barcode(vec![bc("99K00064643", "Code128"), bc("https://x.y/abc", "QRCode")], None);
        assert_eq!(code, "https://x.y/abc");
        // 含 ^ 的 QR 不算
        let code = pick_barcode(vec![bc("99K00064643", "Code128"), bc("a^b^c^d^e^f^g", "QRCode")], None);
        assert_eq!(code, "99K00064643");
    }

    #[test]
    fn 超商_9碼加16碼() {
        let code = pick_barcode(vec![bc("1234567890123456", "Code128"), bc("987654321", "Code39")], None);
        assert_eq!(code, "9876543211234567890123456");
    }

    #[test]
    fn 黑名單與短碼被排除() {
        assert_eq!(pick_barcode(vec![bc("8341748010", "Code128"), bc("12345678", "Code128")], None), NO_READ);
        assert_eq!(pick_barcode(vec![bc("8341748010", "Code128"), bc("13013879313", "Code128")], None), "13013879313");
    }

    #[test]
    fn 同長度取字典序較小的() {
        assert_eq!(pick_barcode(vec![bc("99K00064643", "Code128"), bc("74Z01344756", "Code128")], None), "74Z01344756");
    }

    #[test]
    fn 同幀兩個合法qr_取離畫面中心近的_不看字典序() {
        let center = frame_center(4600, 3400);
        // 9/21 實例：鄰件 74Z01398784 在畫面上緣（字典序較小），這件 74Z01398831 在中央
        let frame = "74Z01398784;(1176,170)(1066,0)(814,83)(921,284);QRCode;74Z01398831;(2518,2443)(2563,2162)(2247,2084)(2190,2373);QRCode@";
        assert_eq!(pick_barcode(parse_frame(frame), center), "74Z01398831");
        // 不知道畫面尺寸 → 舊行為（字典序）
        assert_eq!(pick_barcode(parse_frame(frame), None), "74Z01398784");
        // 只有一個合法 QR 時座標無關
        assert_eq!(pick_barcode(vec![qr_at("X1234567890", 10.0, 10.0), bc("a^b^c^d^e^f^g", "QRCode")], center), "X1234567890");
        // 有座標的優先於沒座標的
        assert_eq!(pick_barcode(vec![bc("A1234567890", "QRCode"), qr_at("B1234567890", 2300.0, 1700.0)], center), "B1234567890");
        // 順豐仍最優先
        assert_eq!(pick_barcode(vec![qr_at("B1234567890", 2300.0, 1700.0), bc("SF1234567890123", "Code128")], center), "SF1234567890123");
    }

    #[test]
    fn 去重擋下的幀當讀碼失敗送出_不是丟掉() {
        let mut d = Dedup { last: String::new(), last_ms: 0, window_ms: 5000 };
        assert_eq!(frame_code(&mut d, "A1234567890".into(), 1000), "A1234567890");
        assert_eq!(frame_code(&mut d, "A1234567890".into(), 2500), NO_READ, "上一件的碼還在畫面裡：這一件沒讀到自己的碼");
        assert_eq!(frame_code(&mut d, "B1234567890".into(), 3000), "B1234567890");
        assert_eq!(frame_code(&mut d, "A1234567890".into(), 9000), "A1234567890", "過了窗口就是新的一件");
    }

    #[test]
    fn 去重_五秒內同碼略過_noread每次都算() {
        let mut d = Dedup { last: String::new(), last_ms: 0, window_ms: 5000 };
        assert!(d.accept("A", 1000));
        assert!(!d.accept("A", 2000));
        assert!(d.accept("B", 2500));
        assert!(d.accept(NO_READ, 2600));
        assert!(d.accept(NO_READ, 2700));
        assert!(d.accept("B", 9000), "超過窗口後同碼可再觸發");
    }
}
