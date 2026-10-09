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

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
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
    /// 閒置探詢：多久沒收到資料就主動送一句無害的查詢，對方一回話閒置計時就重來。
    /// 給停線時完全不說話的裝置（分揀機）用，否則每到 `idle_timeout` 就被當斷線重連一次。
    /// 沒回話會照 `idle_timeout` 的間隔一直問，直到讀逾時把真的斷線抓出來
    pub idle_probe: Option<(Duration, String)>,
    pub keepalive_time: Duration,
    pub keepalive_interval: Duration,
    /// 連上後立即送出的指令（分揀機 `Kx999;Kk2`、皮帶先停止再重置）；用 watch 讓設定改了下次連線就生效
    pub on_connect: watch::Receiver<Vec<String>>,
    /// 程式結束、斷線前送出的指令（皮帶先停止）；連線中才送
    pub on_shutdown: watch::Receiver<Vec<String>>,
    /// 有 `on_shutdown` 的連線要登記在結束流程等的工作裡，否則指令還沒寫出去程序就結束了
    pub tasks: Option<tokio_util::task::TaskTracker>,
    pub reconnect_min: Duration,
    pub reconnect_max: Duration,
}

impl Default for LineOpts {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(1),
            idle_timeout: Some(Duration::from_secs(300)),
            idle_probe: None,
            keepalive_time: Duration::from_secs(10),
            keepalive_interval: Duration::from_secs(5),
            on_connect: fixed_on_connect(Vec::new()),
            on_shutdown: fixed_on_connect(Vec::new()),
            tasks: None,
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

/// 背景工作名稱：崩潰時會出現在現場的警示與事件紀錄，用中文；`name` 本身留給日誌與訊號紀錄
fn task_label(name: &str) -> String {
    let label = match name {
        "belt" => "皮帶線",
        "sorter" => "分揀機",
        other => other,
    };
    format!("{label}連線")
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

    match opts.tasks.clone() {
        Some(tasks) => crate::supervise::spawn_tracked(task_label(name), &tasks, run(name, addr_rx, opts, cancel, cmd_rx, ev_tx, conn_tx)),
        None => crate::supervise::spawn(task_label(name), run(name, addr_rx, opts, cancel, cmd_rx, ev_tx, conn_tx)),
    }

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
        // 解析端 task 已經不在（例如恐慌）：再連上也只是讀到第一行就斷，
        // 而每次連上都會送初始化指令（皮帶「停止＋重置」、分揀機重置）——寧可停在未連線讓畫面看得到
        if ev_tx.is_closed() {
            tracing::error!(device = name, "訊號接收端已關閉，停止連線、不再重連");
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

/// 一行訊號的上限：實際訊號不到 120 位元組，超過這個長度一定是線路雜訊或裝置異常
const MAX_LINE_BYTES: usize = 4096;

/// 寫一條指令的上限
const WRITE_TIMEOUT: Duration = Duration::from_secs(3);

/// 寫一條指令（自動補 `;\n`）。對方不收資料（TCP 視窗塞滿）時 `write_all` 會永遠等下去，
/// 而收發同在一個迴圈：不設上限，讀取也跟著停、斷線永遠判不出來
async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(wr: &mut W, cmd: &str, timeout: Duration) -> Result<(), String> {
    match tokio::time::timeout(timeout, wr.write_all(frame(cmd).as_bytes())).await {
        Ok(r) => r.map_err(|e| e.to_string()),
        Err(_) => Err(format!("{timeout:?} 內寫不出去（逾時）")),
    }
}

/// 讀到的一段資料
#[derive(Debug, PartialEq)]
enum Chunk {
    Line(String),
    /// 超過 `MAX_LINE_BYTES` 還沒換行，整段丟掉；帶丟掉的位元組數
    TooLong(usize),
}

/// 逐行切割。不用 `Lines::next_line` 的原因：它遇到不合法 UTF-8 回錯誤（整條連線被當斷線，
/// 重連時分揀機會被重置），也不限行長（沒有換行的雜訊會一直吃記憶體）。
/// 唯一的 await 是 `fill_buf`（取消安全），讀到一半的行留在 `buf`，放進 `select!` 不會掉資料
struct LineReader<R> {
    inner: BufReader<R>,
    buf: Vec<u8>,
    /// 正在丟棄一段超長資料，直到下一個換行
    discarding: bool,
    dropped: usize,
}

impl<R: AsyncRead + Unpin> LineReader<R> {
    fn new(rd: R) -> Self {
        Self { inner: BufReader::new(rd), buf: Vec::new(), discarding: false, dropped: 0 }
    }

    /// `Ok(None)` = 對方關閉連線
    async fn next(&mut self) -> std::io::Result<Option<Chunk>> {
        loop {
            let avail = self.inner.fill_buf().await?;
            if avail.is_empty() {
                if self.discarding || self.buf.is_empty() {
                    return Ok(None);
                }
                let line = String::from_utf8_lossy(&self.buf).into_owned();
                self.buf.clear();
                return Ok(Some(Chunk::Line(line)));
            }
            match avail.iter().position(|&b| b == b'\n') {
                Some(pos) => {
                    let chunk = if self.discarding {
                        let n = self.dropped + pos;
                        self.discarding = false;
                        self.dropped = 0;
                        Chunk::TooLong(n)
                    } else if self.buf.len() + pos > MAX_LINE_BYTES {
                        let n = self.buf.len() + pos;
                        self.buf.clear();
                        Chunk::TooLong(n)
                    } else {
                        self.buf.extend_from_slice(&avail[..pos]);
                        let line = String::from_utf8_lossy(&self.buf).into_owned();
                        self.buf.clear();
                        Chunk::Line(line)
                    };
                    self.inner.consume(pos + 1);
                    return Ok(Some(chunk));
                }
                None => {
                    let n = avail.len();
                    if self.discarding {
                        self.dropped += n;
                    } else if self.buf.len() + n > MAX_LINE_BYTES {
                        self.dropped = self.buf.len() + n;
                        self.buf.clear();
                        self.discarding = true;
                    } else {
                        self.buf.extend_from_slice(avail);
                    }
                    self.inner.consume(n);
                }
            }
        }
    }
}

/// 程式結束（或沒有人再接收訊號）：斷線前送出 `on_shutdown` 的指令（皮帶先停止）
async fn farewell<W: tokio::io::AsyncWrite + Unpin>(name: &'static str, opts: &LineOpts, wr: &mut W) {
    let cmds = opts.on_shutdown.borrow().clone();
    for cmd in &cmds {
        tracing::info!(device = name, %cmd, "程式結束前送出");
        signal_log::record(name, Dir::Out, cmd);
        if let Err(e) = write_frame(wr, cmd, WRITE_TIMEOUT).await {
            tracing::error!(device = name, %cmd, "程式結束前的指令送不出去: {e}");
            break;
        }
    }
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
    // 讀取必須取消安全：select! 被寫指令分支搶先時，讀到一半的行要留到下一輪接著讀。
    // 若改用 `read_line`，送 Kn 那一刻剛好抵達的 ~c 會整行遺失，接著就是一連串誤判的「收不到 ~c」停線與取消。
    let mut lines = LineReader::new(rd);

    let init_cmds = opts.on_connect.borrow().clone();
    for cmd in &init_cmds {
        tracing::info!(device = name, %cmd, "連線初始化指令");
        signal_log::record(name, Dir::Out, cmd);
        if let Err(e) = write_frame(&mut wr, cmd, WRITE_TIMEOUT).await {
            return format!("初始化指令寫入失敗: {e}");
        }
    }

    let idle = opts.idle_timeout.unwrap_or(Duration::from_secs(365 * 24 * 3600));
    // 讀逾時與閒置探詢都以「最後一次收到資料」起算：送指令不算對方有回應，
    // 否則只要我們自己一直下指令，對方早就死了也永遠抓不到
    let mut last_rx = tokio::time::Instant::now();
    let mut next_probe = opts.idle_probe.as_ref().map(|(after, _)| last_rx + *after);
    let reason: String = loop {
        let probe_at = next_probe.unwrap_or_else(|| last_rx + Duration::from_secs(365 * 24 * 3600));
        tokio::select! {
            _ = cancel.cancelled() => break "服務關閉".into(),
            _ = addr_rx.changed() => break "位址變更，重連".into(),
            _ = tokio::time::sleep_until(probe_at) => {
                let Some((after, cmd)) = opts.idle_probe.as_ref() else { continue };
                tracing::debug!(device = name, %cmd, "閒置探詢");
                signal_log::record(name, Dir::Out, cmd);
                if let Err(e) = write_frame(&mut wr, cmd, WRITE_TIMEOUT).await {
                    break format!("閒置探詢寫入失敗: {e}");
                }
                next_probe = Some(probe_at + *after);
            }
            r = tokio::time::timeout_at(last_rx + idle, lines.next()) => match r {
                Err(_) => break format!("{idle:?} 內無任何資料（讀逾時）"),
                Ok(Ok(None)) => break "對方關閉連線".into(),
                Ok(Ok(Some(Chunk::TooLong(n)))) => {
                    last_rx = tokio::time::Instant::now();
                    next_probe = opts.idle_probe.as_ref().map(|(after, _)| last_rx + *after);
                    tracing::warn!(device = name, bytes = n, "收到超長且沒有換行的資料，已丟棄");
                    signal_log::record(name, Dir::Info, &format!("dropped {n} bytes without newline"));
                }
                Ok(Ok(Some(Chunk::Line(line)))) => {
                    last_rx = tokio::time::Instant::now();
                    next_probe = opts.idle_probe.as_ref().map(|(after, _)| last_rx + *after);
                    let ts_ms = crate::db::now_ms();
                    let text = line.trim_end_matches(['\n', '\r']).to_string();
                    if !text.trim().is_empty() {
                        tracing::trace!(device = name, %text, "收到");
                        signal_log::record(name, Dir::In, &text);
                        if ev_tx.send(LineEvent::Line { text, ts_ms }).await.is_err() {
                            break "事件接收端已關閉".into();
                        }
                    }
                }
                Ok(Err(e)) => break format!("讀取錯誤: {e}"),
            },
            cmd = cmd_rx.recv() => match cmd {
                Some(cmd) => {
                    tracing::debug!(device = name, %cmd, "送出");
                    signal_log::record(name, Dir::Out, &cmd);
                    if let Err(e) = write_frame(&mut wr, &cmd, WRITE_TIMEOUT).await {
                        break format!("寫入失敗: {e}");
                    }
                }
                None => break "指令通道已關閉".into(),
            },
        }
    };
    // 程式關閉時其他通道（位址、指令）會跟著關閉，誰先被處理到不一定：不管從哪一路結束，關閉中都先送結束指令
    if cancel.is_cancelled() {
        farewell(name, opts, &mut wr).await;
        return "服務關閉".into();
    }
    // 沒有人在接收訊號（狀態機崩潰、轉送工作停了）：之後不會再重連，等同關閉。
    // 佇列裡的指令（例如崩潰補救排進來的停皮帶）先寫出去，再送結束指令；不寫的話斷線後會被當成舊連線的指令丟掉
    if ev_tx.is_closed() {
        while let Ok(cmd) = cmd_rx.try_recv() {
            tracing::info!(device = name, %cmd, "接收端已關閉，送出佇列裡的指令");
            signal_log::record(name, Dir::Out, &cmd);
            if let Err(e) = write_frame(&mut wr, &cmd, WRITE_TIMEOUT).await {
                tracing::error!(device = name, %cmd, "接收端已關閉時的指令送不出去: {e}");
                break;
            }
        }
        farewell(name, opts, &mut wr).await;
    }
    reason
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
    async fn 程式結束時先送結束指令再斷線_結束流程等它送完() {
        // 誰先被處理是隨機的，重複幾次才抓得到
        for round in 0..10 {
            let (addr, listener) = fake_device().await;
            let (addr_tx, addr_rx) = watch::channel(addr);
            let cancel = CancellationToken::new();
            let tasks = tokio_util::task::TaskTracker::new();
            let opts = LineOpts {
                on_connect: fixed_on_connect(vec!["KM998 1".into(), "KM998 2".into()]),
                on_shutdown: fixed_on_connect(vec!["KM998 1".into()]),
                tasks: Some(tasks.clone()),
                ..Default::default()
            };
            let (client, mut events) = spawn("test", addr_rx, opts, cancel.clone());
            let (mut s, _) = listener.accept().await.unwrap();
            assert!(matches!(events.recv().await.unwrap(), LineEvent::Connected { .. }));
            let mut got = vec![0u8; b"KM998 1;\nKM998 2;\n".len()];
            s.read_exact(&mut got).await.unwrap();
            assert_eq!(&got[..], b"KM998 1;\nKM998 2;\n");
            // 正式程式結束時：結束通知、位址通道、送指令的一方（狀態機）幾乎同時消失
            cancel.cancel();
            drop(addr_tx);
            drop(client);
            assert!(crate::supervise::drain(&tasks, Duration::from_secs(5)).await, "結束流程要等連線送完結束指令");
            let mut rest = Vec::new();
            let _ = tokio::time::timeout(Duration::from_secs(2), s.read_to_end(&mut rest)).await;
            assert_eq!(String::from_utf8_lossy(&rest), "KM998 1;\n", "第 {round} 次：斷線前先送停止");
        }
    }

    #[tokio::test]
    async fn 訊號接收端消失時_佇列裡的指令與結束指令都要送出() {
        // 包裹追蹤崩潰：它持有的接收端消失，補救動作同時排進一道停皮帶指令；
        // 連線先處理到「下一行送不出去」就斷線的話，那道指令不可被丟掉。誰先被處理是隨機的，重複幾次
        for round in 0..10 {
            let (addr, listener) = fake_device().await;
            let (_addr_tx, addr_rx) = watch::channel(addr);
            let cancel = CancellationToken::new();
            let opts = LineOpts { on_shutdown: fixed_on_connect(vec!["KM998 1".into()]), ..Default::default() };
            let (client, events) = spawn("test", addr_rx, opts, cancel.clone());
            let (mut s, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            drop(events);
            s.write_all(b"~P1 1\n").await.unwrap();
            assert!(client.send("CRASH-STOP"));
            let mut rest = Vec::new();
            let _ = tokio::time::timeout(Duration::from_secs(2), s.read_to_end(&mut rest)).await;
            let text = String::from_utf8_lossy(&rest).into_owned();
            assert!(text.contains("CRASH-STOP;\n"), "第 {round} 次：佇列裡的指令被丟掉：{text:?}");
            assert!(text.ends_with("KM998 1;\n"), "第 {round} 次：沒人接收訊號時要送結束指令：{text:?}");
            cancel.cancel();
        }
    }

    #[test]
    fn 背景工作名稱給現場看_用中文() {
        assert_eq!(task_label("belt"), "皮帶線連線");
        assert_eq!(task_label("sorter"), "分揀機連線");
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

#[cfg(test)]
mod framing_tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    async fn connect_pair() -> (tokio::net::TcpStream, mpsc::Receiver<LineEvent>, CancellationToken, (watch::Sender<String>, LineClient)) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (addr_tx, addr_rx) = watch::channel(l.local_addr().unwrap().to_string());
        let cancel = CancellationToken::new();
        let opts = LineOpts { reconnect_min: Duration::from_millis(20), reconnect_max: Duration::from_millis(20), ..Default::default() };
        let (client, mut events) = spawn("framing", addr_rx, opts, cancel.clone());
        let (s, _) = l.accept().await.unwrap();
        assert!(matches!(events.recv().await.unwrap(), LineEvent::Connected { .. }));
        (s, events, cancel, (addr_tx, client))
    }

    async fn next(events: &mut mpsc::Receiver<LineEvent>) -> LineEvent {
        tokio::time::timeout(Duration::from_secs(2), events.recv()).await.expect("應在 2 秒內收到事件").unwrap()
    }

    #[tokio::test]
    async fn 線路雜訊的不合法位元組_不斷線_該行照轉出_前後行不受影響() {
        let (mut s, mut events, cancel, _a) = connect_pair().await;
        s.write_all(b"~P1 1\n\xff\xfe~X\n~L1 25 111\n").await.unwrap();
        let mut texts = Vec::new();
        for _ in 0..3 {
            match next(&mut events).await {
                LineEvent::Line { text, .. } => texts.push(text),
                other => panic!("不該斷線：{other:?}，已收 {texts:?}"),
            }
        }
        assert_eq!(texts[0], "~P1 1");
        assert!(texts[1].ends_with("~X"), "壞位元組換成替代字元，其餘照轉：{:?}", texts[1]);
        assert_eq!(texts[2], "~L1 25 111");
        cancel.cancel();
    }

    #[tokio::test]
    async fn 沒有換行的超長資料_丟棄後照收下一行_不斷線也不無限吃記憶體() {
        let (mut s, mut events, cancel, _a) = connect_pair().await;
        let junk = vec![b'A'; MAX_LINE_BYTES * 3];
        s.write_all(&junk).await.unwrap();
        s.write_all(b"\n~P2 1\n").await.unwrap();
        match next(&mut events).await {
            LineEvent::Line { text, .. } => assert!(text == "~P2 1", "超長那段要整段丟掉，不能當成一行訊號轉出（收到 {} 位元組）", text.len()),
            other => panic!("不該斷線：{other:?}"),
        }
        cancel.cancel();
    }

    async fn chunks(input: &[u8]) -> Vec<Chunk> {
        let mut r = LineReader::new(input);
        let mut out = Vec::new();
        while let Some(c) = r.next().await.unwrap() {
            out.push(c);
        }
        out
    }

    #[tokio::test]
    async fn 行長邊界_剛好上限照收_多一個位元組整段丟棄_之後照常() {
        let mut input = vec![b'A'; MAX_LINE_BYTES];
        input.push(b'\n');
        input.extend(vec![b'B'; MAX_LINE_BYTES + 1]);
        input.extend_from_slice(b"\n~P3 1\n");
        let got = chunks(&input).await;
        assert_eq!(got.len(), 3, "{:?}", got.iter().map(|c| match c { Chunk::Line(l) => l.len(), Chunk::TooLong(n) => *n }).collect::<Vec<_>>());
        assert_eq!(got[0], Chunk::Line("A".repeat(MAX_LINE_BYTES)));
        assert_eq!(got[1], Chunk::TooLong(MAX_LINE_BYTES + 1));
        assert_eq!(got[2], Chunk::Line("~P3 1".into()));
    }

    #[tokio::test]
    async fn 連線結束時沒有換行的最後半行照轉出_丟棄中的超長段不轉出() {
        assert_eq!(chunks(b"~P1 1\n~E1 1").await, vec![Chunk::Line("~P1 1".into()), Chunk::Line("~E1 1".into())]);
        let mut junk = vec![b'Z'; MAX_LINE_BYTES * 2];
        junk.splice(0..0, b"~P1 1\n".iter().copied());
        assert_eq!(chunks(&junk).await, vec![Chunk::Line("~P1 1".into())]);
    }

    #[tokio::test]
    async fn 一行分兩段到達_中間插入送指令_不掉資料() {
        let (mut s, mut events, cancel, (_a, client)) = connect_pair().await;
        s.write_all(b"~c2 2 4").await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(client.send("Kn 101 01 03 100 23 1"), "送指令讓 select! 先走寫入分支");
        let mut got = [0u8; 64];
        let n = tokio::io::AsyncReadExt::read(&mut s, &mut got).await.unwrap();
        assert_eq!(&got[..n], b"Kn 101 01 03 100 23 1;\n");
        s.write_all(b"10 1\n").await.unwrap();
        match next(&mut events).await {
            LineEvent::Line { text, .. } => assert_eq!(text, "~c2 2 410 1", "前半段要留著接上後半段"),
            other => panic!("{other:?}"),
        }
        cancel.cancel();
    }

    #[tokio::test]
    async fn 事件接收端已不在時_不再重連_不會一再送初始化指令() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (_addr_tx, addr_rx) = watch::channel(l.local_addr().unwrap().to_string());
        let cancel = CancellationToken::new();
        let opts = LineOpts {
            on_connect: fixed_on_connect(vec!["KM998 1".into(), "KM999".into()]),
            reconnect_min: Duration::from_millis(20),
            reconnect_max: Duration::from_millis(20),
            ..Default::default()
        };
        let (_client, events) = spawn("orphan", addr_rx, opts, cancel.clone());
        let (mut s, _) = l.accept().await.unwrap();
        // 解析端 task 死掉（例如恐慌）：接收端被丟掉
        drop(events);
        s.write_all(b"~P1 1\n").await.unwrap();
        let again = tokio::time::timeout(Duration::from_millis(600), l.accept()).await;
        assert!(again.is_err(), "接收端已不在還重連：每次重連都會再送一次停止＋重置");
        cancel.cancel();
    }

    /// 永遠寫不進去的對端（裝置不收資料、TCP 視窗塞滿）
    struct StuckWriter;
    impl tokio::io::AsyncWrite for StuckWriter {
        fn poll_write(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>, _: &[u8]) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Pending
        }
        fn poll_flush(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Pending
        }
        fn poll_shutdown(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Pending
        }
    }

    #[tokio::test]
    async fn 對端不收資料時寫入會逾時_不會讓收發迴圈永久卡住() {
        let mut w = StuckWriter;
        let r = tokio::time::timeout(Duration::from_secs(2), write_frame(&mut w, "Kn 101 01 03 100 23 1", Duration::from_millis(200))).await;
        match r {
            Ok(Err(e)) => assert!(e.contains("逾時"), "{e}"),
            Ok(Ok(())) => panic!("不可能寫成功"),
            Err(_) => panic!("寫入沒有逾時：2 秒後仍卡著"),
        }
    }
}

#[cfg(test)]
mod probe_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn 閒置時主動探詢_對方回話就不算斷線_不回話才讀逾時() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let (_addr_tx, addr_rx) = watch::channel(addr);
        let cancel = CancellationToken::new();
        let opts = LineOpts {
            idle_timeout: Some(Duration::from_millis(700)),
            idle_probe: Some((Duration::from_millis(200), "Kd[".into())),
            reconnect_min: Duration::from_millis(50),
            reconnect_max: Duration::from_millis(50),
            ..Default::default()
        };
        let (_client, mut events) = spawn("probe", addr_rx, opts, cancel.clone());
        let (mut s, _) = l.accept().await.unwrap();
        assert!(matches!(events.recv().await.unwrap(), LineEvent::Connected { .. }));

        // 200ms 沒資料 → 收到探詢；回一句話 → 閒置計時重來
        let mut buf = [0u8; 64];
        let n = tokio::time::timeout(Duration::from_millis(500), s.read(&mut buf)).await.unwrap().unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).contains("Kd["), "閒置該送探詢");
        s.write_all(b"~[00000000]\n").await.unwrap();
        assert!(matches!(events.recv().await.unwrap(), LineEvent::Line { .. }));

        // 之後裝死：探詢會一直問（每 200ms），700ms 沒回話才判斷線
        let started = tokio::time::Instant::now();
        let mut probes = 0;
        loop {
            match tokio::time::timeout(Duration::from_millis(1500), events.recv()).await.unwrap().unwrap() {
                LineEvent::Disconnected { reason, .. } => {
                    assert!(reason.contains("讀逾時"), "{reason}");
                    break;
                }
                _ => {}
            }
        }
        while let Ok(Ok(n)) = tokio::time::timeout(Duration::from_millis(50), s.read(&mut buf)).await {
            if n == 0 { break }
            probes += String::from_utf8_lossy(&buf[..n]).matches("Kd[").count();
        }
        assert!(started.elapsed() >= Duration::from_millis(600), "要等滿讀逾時才判斷線");
        assert!(probes >= 2, "斷線前應該問過好幾次，實際 {probes}");
        cancel.cancel();
    }
}
