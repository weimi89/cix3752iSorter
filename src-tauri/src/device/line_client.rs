//! 通用文字行 TCP client：連線、斷線偵測、退避重連、取消。
//!
//! 皮帶線、分揀機都是「一行一訊號、`\n` 結尾」的裝置，差別只在解析；
//! 這裡負責連線生命週期，解析交給各裝置模組。
//!
//! 設計重點（每一條都是舊系統實際出過事的地方）：
//! - 連線句柄只活在本 task 內，外部只能透過 channel 送指令 → 不會有「斷線後拿到 nil 連線就寫」的當機。
//! - 一條連線一個 `select!` 迴圈，斷線即整個 task 退出重來 → 不會有舊控制器殘留與新連線搶事件。
//! - TCP keepalive + 讀逾時 → 對方無聲消失（電源重開、線被拔）在秒級被發現，而不是永遠卡在讀。
//! - 斷線時丟棄尚未送出的指令：那些指令是針對已經不存在的連線狀態下的，重連後再送只會亂。

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use super::signal_log::{self, Dir};
use crate::protocol::command::frame;

#[derive(Clone, Debug)]
pub enum LineEvent {
    Connected { addr: String },
    Disconnected { addr: String, reason: String },
    Line { text: String, ts_ms: i64 },
}

#[derive(Clone, Debug)]
pub struct LineOpts {
    pub connect_timeout: Duration,
    /// 多久沒收到任何資料視為斷線；None = 只靠 TCP keepalive
    pub idle_timeout: Option<Duration>,
    pub keepalive_time: Duration,
    pub keepalive_interval: Duration,
    /// 連上後立即送出的指令（分揀機 `Kx999;Kk2`、皮帶先停止再重置）；用 watch 讓設定改了下次連線就生效
    pub on_connect: watch::Receiver<Vec<String>>,
    pub reconnect_min: Duration,
    pub reconnect_max: Duration,
}

impl Default for LineOpts {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(1),
            idle_timeout: Some(Duration::from_secs(300)),
            keepalive_time: Duration::from_secs(10),
            keepalive_interval: Duration::from_secs(5),
            on_connect: fixed_on_connect(Vec::new()),
            reconnect_min: Duration::from_secs(1),
            reconnect_max: Duration::from_secs(5),
        }
    }
}

/// 固定不變的初始化指令清單（測試或不吃設定的裝置用）
pub fn fixed_on_connect(cmds: Vec<String>) -> watch::Receiver<Vec<String>> {
    // sender 丟掉後 receiver 仍 borrow 得到初值；這裡從不 await changed()
    watch::channel(cmds).1
}

/// 對外的指令入口。`send` 在未連線時直接回 false，呼叫端自行決定要記錄或告警。
#[derive(Clone)]
pub struct LineClient {
    name: &'static str,
    cmd_tx: mpsc::Sender<String>,
    connected: watch::Receiver<bool>,
}

impl LineClient {
    pub fn is_connected(&self) -> bool {
        *self.connected.borrow()
    }

    pub fn connected_watch(&self) -> watch::Receiver<bool> {
        self.connected.clone()
    }

    /// 排入送出；未連線或佇列滿時回 false（不阻塞呼叫端的即時迴圈）。
    pub fn send(&self, cmd: impl Into<String>) -> bool {
        let cmd = cmd.into();
        if !self.is_connected() {
            tracing::warn!(device = self.name, %cmd, "未連線，指令丟棄");
            return false;
        }
        match self.cmd_tx.try_send(cmd) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(device = self.name, "指令佇列滿或已關閉，指令丟棄: {e}");
                false
            }
        }
    }
}

/// 啟動連線 task。`addr_rx` 變動時會斷開重連到新位址。
pub fn spawn(
    name: &'static str,
    addr_rx: watch::Receiver<String>,
    opts: LineOpts,
    cancel: CancellationToken,
) -> (LineClient, mpsc::Receiver<LineEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<String>(256);
    let (ev_tx, ev_rx) = mpsc::channel::<LineEvent>(1024);
    let (conn_tx, conn_rx) = watch::channel(false);

    tokio::spawn(run(name, addr_rx, opts, cancel, cmd_rx, ev_tx, conn_tx));

    (LineClient { name, cmd_tx, connected: conn_rx }, ev_rx)
}

async fn run(
    name: &'static str,
    mut addr_rx: watch::Receiver<String>,
    opts: LineOpts,
    cancel: CancellationToken,
    mut cmd_rx: mpsc::Receiver<String>,
    ev_tx: mpsc::Sender<LineEvent>,
    conn_tx: watch::Sender<bool>,
) {
    let mut backoff = opts.reconnect_min;
    let mut last_fail_log = std::time::Instant::now() - Duration::from_secs(3600);

    loop {
        if cancel.is_cancelled() {
            break;
        }
        let addr = addr_rx.borrow_and_update().clone();

        let connect = tokio::time::timeout(opts.connect_timeout, TcpStream::connect(&addr));
        let stream = tokio::select! {
            _ = cancel.cancelled() => break,
            r = connect => match r {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    // 裝置關機時每秒都會失敗一次；同一原因 60 秒只記一次，避免日誌被淹沒
                    if last_fail_log.elapsed() >= Duration::from_secs(60) {
                        tracing::warn!(device = name, %addr, "連線失敗: {e}");
                        last_fail_log = std::time::Instant::now();
                    }
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = tokio::time::sleep(backoff) => {}
                        _ = addr_rx.changed() => { backoff = opts.reconnect_min; }
                    }
                    backoff = (backoff * 2).min(opts.reconnect_max);
                    continue;
                }
                Err(_) => {
                    if last_fail_log.elapsed() >= Duration::from_secs(60) {
                        tracing::warn!(device = name, %addr, "連線逾時");
                        last_fail_log = std::time::Instant::now();
                    }
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = tokio::time::sleep(backoff) => {}
                        _ = addr_rx.changed() => { backoff = opts.reconnect_min; }
                    }
                    backoff = (backoff * 2).min(opts.reconnect_max);
                    continue;
                }
            },
        };

        if let Err(e) = configure_socket(&stream, &opts) {
            tracing::warn!(device = name, "設定 keepalive 失敗（繼續使用）: {e}");
        }
        tracing::info!(device = name, %addr, "連線成功");
        signal_log::record(name, Dir::Info, &format!("connected {addr}"));
        backoff = opts.reconnect_min;
        last_fail_log = std::time::Instant::now() - Duration::from_secs(3600);
        let _ = conn_tx.send(true);
        let _ = ev_tx.send(LineEvent::Connected { addr: addr.clone() }).await;

        let reason = serve_connection(name, stream, &opts, &cancel, &mut cmd_rx, &ev_tx, &mut addr_rx).await;

        let _ = conn_tx.send(false);
        // 丟掉斷線期間累積的指令：它們對應的是舊連線的狀態
        let mut dropped = 0;
        while cmd_rx.try_recv().is_ok() {
            dropped += 1;
        }
        tracing::warn!(device = name, %addr, dropped, "連線結束: {reason}");
        signal_log::record(name, Dir::Info, &format!("disconnected {addr}: {reason}"));
        let _ = ev_tx.send(LineEvent::Disconnected { addr: addr.clone(), reason }).await;

        if cancel.is_cancelled() {
            break;
        }
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(opts.reconnect_min) => {}
        }
    }
    let _ = conn_tx.send(false);
    tracing::info!(device = name, "連線 task 結束");
}

fn configure_socket(stream: &TcpStream, opts: &LineOpts) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let sock = socket2::SockRef::from(stream);
    let ka = socket2::TcpKeepalive::new()
        .with_time(opts.keepalive_time)
        .with_interval(opts.keepalive_interval);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let ka = ka.with_retries(3);
    sock.set_tcp_keepalive(&ka)
}

/// 單一條連線的收發迴圈；回傳結束原因。
async fn serve_connection(
    name: &'static str,
    stream: TcpStream,
    opts: &LineOpts,
    cancel: &CancellationToken,
    cmd_rx: &mut mpsc::Receiver<String>,
    ev_tx: &mpsc::Sender<LineEvent>,
    addr_rx: &mut watch::Receiver<String>,
) -> String {
    let (rd, mut wr) = stream.into_split();
    // 讀取必須用 cancel-safe 的 `Lines::next_line`：select! 被寫指令分支搶先時，
    // 讀到一半的行留在內部緩衝、下一輪接著讀。若改用 `read_line`，送 Kn 那一刻剛好
    // 抵達的 ~c 會整行遺失，接著就是一連串誤判的「收不到 ~c」停線與取消。
    let mut lines = BufReader::new(rd).lines();

    let init_cmds = opts.on_connect.borrow().clone();
    for cmd in &init_cmds {
        tracing::info!(device = name, %cmd, "連線初始化指令");
        signal_log::record(name, Dir::Out, cmd);
        if let Err(e) = wr.write_all(frame(cmd).as_bytes()).await {
            return format!("初始化指令寫入失敗: {e}");
        }
    }

    loop {
        let idle = opts.idle_timeout.unwrap_or(Duration::from_secs(365 * 24 * 3600));
        tokio::select! {
            _ = cancel.cancelled() => return "服務關閉".into(),
            _ = addr_rx.changed() => return "位址變更，重連".into(),
            r = tokio::time::timeout(idle, lines.next_line()) => match r {
                Err(_) => return format!("{idle:?} 內無任何資料（讀逾時）"),
                Ok(Ok(None)) => return "對方關閉連線".into(),
                Ok(Ok(Some(line))) => {
                    let ts_ms = crate::db::now_ms();
                    let text = line.trim_end_matches(['\n', '\r']).to_string();
                    if !text.trim().is_empty() {
                        tracing::trace!(device = name, %text, "收到");
                        signal_log::record(name, Dir::In, &text);
                        if ev_tx.send(LineEvent::Line { text, ts_ms }).await.is_err() {
                            return "事件接收端已關閉".into();
                        }
                    }
                }
                Ok(Err(e)) => return format!("讀取錯誤: {e}"),
            },
            cmd = cmd_rx.recv() => match cmd {
                Some(cmd) => {
                    tracing::debug!(device = name, %cmd, "送出");
                    signal_log::record(name, Dir::Out, &cmd);
                    if let Err(e) = wr.write_all(frame(&cmd).as_bytes()).await {
                        return format!("寫入失敗: {e}");
                    }
                }
                None => return "指令通道已關閉".into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    /// 起一個假裝置：接受連線、把收到的東西記下來、可以主動掛斷。
    async fn fake_device() -> (String, TcpListener) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap().to_string();
        (addr, l)
    }

    #[tokio::test]
    async fn 連線_收行_送指令_斷線重連() {
        let (addr, listener) = fake_device().await;
        let (addr_tx, addr_rx) = watch::channel(addr.clone());
        let cancel = CancellationToken::new();
        let opts = LineOpts {
            on_connect: fixed_on_connect(vec!["Kx999;Kk2".into()]),
            reconnect_min: Duration::from_millis(50),
            reconnect_max: Duration::from_millis(100),
            ..Default::default()
        };
        let (client, mut events) = spawn("test", addr_rx, opts, cancel.clone());

        // 第一條連線
        let (mut s, _) = listener.accept().await.unwrap();
        assert!(matches!(events.recv().await.unwrap(), LineEvent::Connected { .. }));
        let mut got = [0u8; 64];
        let n = s.read(&mut got).await.unwrap();
        assert_eq!(&got[..n], b"Kx999;Kk2;\n", "連上後應先送初始化指令");

        s.write_all(b"~P24 1\n~L24 25 111\n").await.unwrap();
        match events.recv().await.unwrap() {
            LineEvent::Line { text, .. } => assert_eq!(text, "~P24 1"),
            other => panic!("{other:?}"),
        }
        match events.recv().await.unwrap() {
            LineEvent::Line { text, .. } => assert_eq!(text, "~L24 25 111"),
            other => panic!("{other:?}"),
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(client.is_connected());
        assert!(client.send("KM998 3"));
        let n = s.read(&mut got).await.unwrap();
        assert_eq!(&got[..n], b"KM998 3;\n");

        // 裝置端掛斷 → 應收到 Disconnected，之後自動重連
        drop(s);
        assert!(matches!(events.recv().await.unwrap(), LineEvent::Disconnected { .. }));
        let (_s2, _) = listener.accept().await.unwrap();
        assert!(matches!(events.recv().await.unwrap(), LineEvent::Connected { .. }));

        drop(addr_tx);
        cancel.cancel();
    }

    #[tokio::test]
    async fn 未連線時指令直接丟棄不阻塞() {
        let (_addr_tx, addr_rx) = watch::channel("127.0.0.1:1".to_string());
        let cancel = CancellationToken::new();
        let (client, _events) = spawn("test", addr_rx, LineOpts::default(), cancel.clone());
        assert!(!client.send("KM998 1"));
        cancel.cancel();
    }

    #[tokio::test]
    async fn 讀逾時視為斷線() {
        let (addr, listener) = fake_device().await;
        let (_addr_tx, addr_rx) = watch::channel(addr);
        let cancel = CancellationToken::new();
        let opts = LineOpts {
            idle_timeout: Some(Duration::from_millis(80)),
            reconnect_min: Duration::from_millis(20),
            ..Default::default()
        };
        let (_client, mut events) = spawn("test", addr_rx, opts, cancel.clone());
        let (_s, _) = listener.accept().await.unwrap();
        assert!(matches!(events.recv().await.unwrap(), LineEvent::Connected { .. }));
        match events.recv().await.unwrap() {
            LineEvent::Disconnected { reason, .. } => assert!(reason.contains("讀逾時"), "{reason}"),
            other => panic!("{other:?}"),
        }
        cancel.cancel();
    }
}
