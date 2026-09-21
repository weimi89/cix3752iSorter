//! 包裹狀態機 task：單一 owner，裝置事件、格口結果、定時 tick、網頁指令全部匯進來依序處理。

pub mod machine;
pub mod parcel;
pub mod store;

#[cfg(test)]
mod machine_tests;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::AppState;
use crate::device::{Device, DeviceEvent, LineClient};
use crate::event_bus;
use crate::event_log::{self, Level};
use crate::protocol::Cid;

pub use machine::{ChuteRow, Input, Machine};
pub use parcel::{ChuteSource, LabelPayload, Parcel, Status};

use crate::label::LabelJob;

pub struct Devices {
    pub belt: LineClient,
    pub sorter: LineClient,
}

/// 格口解析請求：狀態機丟出去，`chute::ChuteResolver` 之後把結果送回 `Input::Chute`。
#[derive(Clone, Debug)]
pub struct ChuteRequest {
    pub key: u64,
    pub ulid: String,
    pub barcode: String,
    /// NoRead：只是讓中介機拍照存證與計數，本機已決定預設口，回覆不必送回狀態機
    pub notify_only: bool,
}

/// 給網頁層／其他模組的控制入口
#[derive(Clone)]
pub struct TrackerHandle {
    tx: mpsc::Sender<Input>,
    query_tx: mpsc::Sender<oneshot::Sender<TrackerSnapshot>>,
    sorter: LineClient,
    ir: Arc<IrChannel>,
}

/// 光電檢查的「問一句、等一句」：網頁送 `Kd[`／`p1` 後在這裡等分揀機回覆；
/// 一次只做一件（`op` 鎖），回覆由狀態機 task 攔下來送進 `pending`
pub struct IrChannel {
    op: tokio::sync::Mutex<()>,
    pending: std::sync::Mutex<IrPending>,
}

#[derive(Default)]
struct IrPending {
    kd: Option<oneshot::Sender<String>>,
    p1: Option<(oneshot::Sender<Vec<String>>, Vec<String>)>,
}

impl IrChannel {
    /// 分揀機的行進來時呼叫：是不是光電查詢的回覆
    fn on_sorter_signal(&self, sig: &crate::protocol::SorterSignal) {
        use crate::protocol::SorterSignal;
        let mut p = self.pending.lock().unwrap();
        match sig {
            SorterSignal::IrStatus(body) => {
                if let Some(tx) = p.kd.take() {
                    let _ = tx.send(body.clone());
                }
            }
            SorterSignal::P1Line(line) => {
                if let Some((_, lines)) = p.p1.as_mut() {
                    lines.push(line.clone());
                    if line.contains("FFFFFFFF") {
                        if let Some((tx, lines)) = p.p1.take() {
                            let _ = tx.send(lines);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// 光電查詢等分揀機回覆的上限；舊程式是 300ms × 4 次
const IR_REPLY_TIMEOUT: Duration = Duration::from_millis(1500);
/// 進維修模式後要停一下再下指令（舊程式 sleep 100ms）
const IR_SETTLE: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, serde::Serialize)]
pub struct TrackerSnapshot {
    pub belt_running: bool,
    pub head_current: Option<u64>,
    pub head_queue: usize,
    pub head_jam: bool,
    pub blocked: bool,
    pub today_count: u64,
    pub counters: machine::Counters,
    pub current: Option<Parcel>,
    pub in_flight: Vec<Parcel>,
    /// 產生這份快照時的伺服器時鐘。前端算「已經過」要用它對齊自己的時鐘,
    /// 網頁版從別台電腦開時兩邊時鐘常差個一兩秒,直接用瀏覽器時間會算出負數。
    pub now_ms: i64,
}

impl TrackerHandle {
    pub async fn belt_start(&self) {
        let _ = self.tx.send(Input::BeltStart).await;
    }
    pub async fn belt_stop(&self) {
        let _ = self.tx.send(Input::BeltStop).await;
    }
    pub async fn sorter_reset(&self) {
        let _ = self.tx.send(Input::SorterReset).await;
    }
    pub async fn sorter_raw(&self, cmd: String) {
        let _ = self.tx.send(Input::SorterRaw(cmd)).await;
    }
    pub fn chute_result(&self, key: u64, code: String, cid: Cid, source: ChuteSource, response_id: Option<i64>, reason: Option<String>, label: Option<LabelPayload>) {
        // 送不進去只會讓那件在 ~O 時走預設口，但要留下痕跡，現場才查得到「面單其實抓到了」
        if self.tx.try_send(Input::Chute { key, code: code.clone(), cid, source, response_id, reason, label }).is_err() {
            tracing::error!(key, %code, "狀態機佇列滿，格口結果丟棄（該件將走預設口）");
        }
    }
    pub async fn snapshot(&self) -> Option<TrackerSnapshot> {
        let (tx, rx) = oneshot::channel();
        self.query_tx.send(tx).await.ok()?;
        rx.await.ok()
    }

    fn ensure_sorter(&self) -> Result<(), String> {
        if self.sorter.is_connected() { Ok(()) } else { Err("分揀機未連線".into()) }
    }

    /// `Kd[`：每台分揀機的光電總狀態（`~[…]` 原文）
    pub async fn ir_status(&self) -> Result<String, String> {
        let _op = self.ir.op.lock().await;
        self.ensure_sorter()?;
        let (tx, rx) = oneshot::channel();
        self.ir.pending.lock().unwrap().kd = Some(tx);
        self.sorter.send(crate::protocol::ir::query_status());
        match tokio::time::timeout(IR_REPLY_TIMEOUT, rx).await {
            Ok(Ok(body)) => Ok(body),
            _ => {
                self.ir.pending.lock().unwrap().kd = None;
                Err("分揀機沒有回覆光電狀態".into())
            }
        }
    }

    /// 單台每顆光電讀值（進維修模式 → `p1` → 離開維修模式）
    pub async fn ir_detail(&self, m2: u32) -> Result<Vec<String>, String> {
        use crate::protocol::ir;
        let _op = self.ir.op.lock().await;
        self.ensure_sorter()?;
        self.sorter.send(ir::enter_maintenance());
        tokio::time::sleep(IR_SETTLE).await;
        let (tx, rx) = oneshot::channel();
        self.ir.pending.lock().unwrap().p1 = Some((tx, Vec::new()));
        self.sorter.send(ir::query_p1(m2));
        let r = match tokio::time::timeout(IR_REPLY_TIMEOUT, rx).await {
            Ok(Ok(lines)) => Ok(lines),
            _ => {
                self.ir.pending.lock().unwrap().p1 = None;
                Err("分揀機沒有回覆光電讀值".into())
            }
        };
        self.sorter.send(ir::leave_maintenance());
        r
    }

    /// 屏蔽／解除屏蔽整台的光電（光電壞了先屏蔽讓線能跑，修好再解除）
    pub async fn ir_block(&self, m2: u32, block: bool) -> Result<(), String> {
        use crate::protocol::ir;
        let _op = self.ir.op.lock().await;
        self.ensure_sorter()?;
        self.sorter.send(ir::enter_maintenance());
        tokio::time::sleep(IR_SETTLE).await;
        self.sorter.send(ir::block(m2, block));
        tokio::time::sleep(IR_SETTLE).await;
        self.sorter.send(ir::leave_maintenance());
        Ok(())
    }
}

pub async fn load_chutes(db: &crate::db::DbPool) -> anyhow::Result<HashMap<String, ChuteRow>> {
    let rows: Vec<(String, i64, Option<String>, i64, String, i64)> =
        sqlx::query_as("SELECT code, cid, printer_port, enabled, label, sort_order FROM chutes").fetch_all(db).await?;
    Ok(rows
        .into_iter()
        .map(|(code, cid, printer_port, enabled, label, sort_order)| {
            (code.clone(), ChuteRow { code, cid: Cid(cid as u32), printer_port, enabled: enabled != 0, label, sort_order })
        })
        .collect())
}

async fn today_count(db: &crate::db::DbPool) -> u64 {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM parcels WHERE started_at >= ?")
        .bind(format!("{today} 00:00:00"))
        .fetch_one(db)
        .await
        .unwrap_or(0) as u64
}

/// 真正的副作用出口：接裝置、DB、事件匯流排、格口解析器
struct LiveOutputs {
    app: AppState,
    devices: Devices,
    store: store::Store,
    report_queue: crate::middleware::report_queue::ReportQueue,
    chute_tx: mpsc::Sender<ChuteRequest>,
    label_tx: mpsc::Sender<LabelJob>,
    led_via_sorter: bool,
    jam: JamThrottle,
    jam_hot: JamHotspot,
    jam_tx: mpsc::Sender<JamAlert>,
}

/// 要請中介機出聲的現場告警：卡件單次（照舊 Node 節流）、卡件熱點
#[derive(Debug, Clone, PartialEq)]
pub enum JamAlert {
    Chute(i32),
    Hotspot { module: i32, count: usize },
}

/// 卡件熱點：同一模組在 `window_ms` 內累積到 `min_count` 次就提示「請檢查機構」，
/// 提示過後 `realert_ms` 內不再重複。2026-09-18 模組 3 一天卡 11 次（全天 25 次），
/// 單次告警每次都響、沒人看得出是同一個地方一直卡
pub struct JamHotspot {
    window_ms: i64,
    min_count: usize,
    realert_ms: i64,
    hits: HashMap<i32, std::collections::VecDeque<i64>>,
    last_alert: HashMap<i32, i64>,
}

impl JamHotspot {
    pub fn new(window_ms: i64, min_count: usize, realert_ms: i64) -> Self {
        Self { window_ms, min_count, realert_ms, hits: HashMap::new(), last_alert: HashMap::new() }
    }

    /// `~k` 的 pos → 模組（`pos/10+1`，與 JamThrottle 同算法）；回 Some((模組, 一小時內次數)) 表示要提示
    pub fn on_jam(&mut self, pos: i32, now: i64) -> Option<(i32, usize)> {
        let module = pos / 10 + 1;
        let q = self.hits.entry(module).or_default();
        q.push_back(now);
        while q.front().is_some_and(|&t| now - t > self.window_ms) {
            q.pop_front();
        }
        let count = q.len();
        if count < self.min_count {
            return None;
        }
        let last = self.last_alert.get(&module).copied().unwrap_or(i64::MIN / 2);
        if now - last < self.realert_ms {
            return None;
        }
        self.last_alert.insert(module, now);
        Some((module, count))
    }
}

/// 卡件告警節流（對齊舊 Node）：同一格口 `throttle_ms` 內只報一次；
/// 但連續 `reset_ms` 沒再收到該格口的 `~k` 就視為那次堵塞結束，下一個 `~k` 立刻再報
pub struct JamThrottle {
    throttle_ms: i64,
    reset_ms: i64,
    /// 格口 → (最後一次告警, 最後一次收到 ~k)
    state: HashMap<i32, (i64, i64)>,
}

impl JamThrottle {
    pub fn new(throttle_ms: i64, reset_ms: i64) -> Self {
        Self { throttle_ms, reset_ms, state: HashMap::new() }
    }

    /// 設定改了只換參數，正在節流中的格口不重算（否則存個設定就會多報一次）
    pub fn set_params(&mut self, throttle_ms: i64, reset_ms: i64) {
        self.throttle_ms = throttle_ms;
        self.reset_ms = reset_ms;
    }

    /// `~k` 的 pos → 格口號（`pos/10+1`）；回 Some 表示要告警
    pub fn on_signal(&mut self, pos: i32, now: i64) -> Option<i32> {
        let chute_no = pos / 10 + 1;
        let (mut last_alert, last_signal) = self.state.get(&chute_no).copied().unwrap_or((i64::MIN / 2, i64::MIN / 2));
        if now - last_signal > self.reset_ms {
            last_alert = i64::MIN / 2;
        }
        let fire = now - last_alert >= self.throttle_ms;
        self.state.insert(chute_no, (if fire { now } else { last_alert }, now));
        fire.then_some(chute_no)
    }
}

impl machine::Outputs for LiveOutputs {
    fn belt_cmd(&mut self, cmd: &str) {
        self.devices.belt.send(cmd);
    }
    fn sorter_cmd(&mut self, cmd: &str) {
        self.devices.sorter.send(cmd);
    }
    fn led_cmd(&mut self, cmd: &str) {
        if self.led_via_sorter { self.devices.sorter.send(cmd) } else { self.devices.belt.send(cmd) };
    }
    fn store_insert(&mut self, p: &Parcel) {
        self.store.insert(p);
    }
    fn store_update(&mut self, p: &Parcel) {
        self.store.update(p);
    }
    fn store_event(&mut self, p: &Parcel, ts_ms: i64, source: &'static str, kind: &str, raw: Option<&str>) {
        self.store.event(p, ts_ms, source, kind, raw.map(String::from));
    }
    fn store_forget(&mut self, p: &Parcel) {
        self.store.forget(p);
    }
    fn store_daily(&mut self, p: &Parcel) {
        self.store.send(store::StoreOp::Daily(p.clone()));
    }
    fn request_chute(&mut self, p: &Parcel) {
        let barcode = p.barcode_or_noread().to_string();
        let notify_only = barcode == crate::device::camera::NO_READ;
        let req = ChuteRequest { key: p.key, ulid: p.ulid.clone(), barcode, notify_only };
        if self.chute_tx.try_send(req).is_err() {
            tracing::error!("格口解析佇列滿，條碼 {} 將走預設口", p.barcode_or_noread());
        }
    }
    fn print_label(&mut self, p: &Parcel, printer_port: Option<String>, label: LabelPayload) {
        let Some(chute) = p.chute.as_ref() else { return };
        let job = LabelJob { parcel_ulid: p.ulid.clone(), barcode: p.barcode_or_noread().to_string(), chute_code: chute.code.clone(), printer_port, label };
        if self.label_tx.try_send(job).is_err() {
            event_log::log(&self.app.db, Level::Error, "printer", "label", format!("{} 條碼 {} 列印佇列滿，面單丟棄", chute.code, p.barcode_or_noread()));
        }
    }
    fn parcel_changed(&mut self, p: &Parcel) {
        event_bus::emit("parcel-updated", p);
    }
    fn log(&mut self, level: Level, category: &'static str, action: &'static str, msg: String) {
        event_log::log(&self.app.db, level, category, action, msg);
    }
    fn jam_alert(&mut self, pos: i32) {
        if let Some(chute_no) = self.jam.on_signal(pos, crate::db::now_ms()) {
            let _ = self.jam_tx.try_send(JamAlert::Chute(chute_no));
        }
    }
    fn store_jam(&mut self, ts_ms: i64, cart: u32, pos: i32, parcel: Option<&Parcel>) {
        // 每次堵塞開始只記一筆（machine 已去重），熱點就在這裡數
        if let Some((module, count)) = self.jam_hot.on_jam(pos, ts_ms) {
            let _ = self.jam_tx.try_send(JamAlert::Hotspot { module, count });
        }
        self.store.send(store::StoreOp::Jam {
            ts_ms,
            cart,
            pos,
            ulid: parcel.map(|p| p.ulid.clone()),
            barcode: parcel.map(|p| p.barcode_or_noread().to_string()),
            chute_code: parcel.and_then(|p| p.chute.as_ref().map(|c| c.code.clone())),
        });
    }
    fn chute_decided(&mut self, p: &Parcel) {
        event_bus::emit("chute-decided", p);
        // 回報中介機「已收到格口」：有 response_id 才回報（NoRead／null 不回報，契約如此）
        if let Some(rid) = p.chute.as_ref().and_then(|c| c.response_id) {
            let q = self.report_queue.clone();
            let ulid = p.ulid.clone();
            let db = self.app.db.clone();
            tokio::spawn(async move {
                if let Err(e) = q.enqueue(&ulid, rid).await {
                    event_log::log(&db, Level::Error, "middleware", "report_enqueue", format!("回報入列失敗 response_id={rid}: {e}"));
                }
            });
        }
    }
}

pub struct TrackerPorts {
    /// 格口查詢請求（M3 的中介機 client 接這裡）
    pub chute_rx: mpsc::Receiver<ChuteRequest>,
    /// 卡件告警（單次格口號 / 熱點）
    pub jam_rx: mpsc::Receiver<JamAlert>,
}

pub async fn spawn(
    app: AppState,
    devices: Devices,
    mut rx: mpsc::Receiver<DeviceEvent>,
    label_tx: mpsc::Sender<LabelJob>,
    cancel: CancellationToken,
) -> anyhow::Result<(TrackerHandle, TrackerPorts)> {
    let closed = store::close_orphans(&app.db).await?;
    if closed > 0 {
        event_log::log(&app.db, Level::Warn, "tracker", "orphans", format!("上次未收尾的在途件 {closed} 筆已標為失去追蹤"));
    }
    let chutes = load_chutes(&app.db).await?;
    let today = today_count(&app.db).await;
    let cfg = app.config.current();

    let (tx, mut in_rx) = mpsc::channel::<Input>(1024);
    let (query_tx, mut query_rx) = mpsc::channel::<oneshot::Sender<TrackerSnapshot>>(16);
    let (chute_tx, chute_rx) = mpsc::channel::<ChuteRequest>(256);
    let (jam_tx, jam_rx) = mpsc::channel::<JamAlert>(64);
    let ir = Arc::new(IrChannel { op: tokio::sync::Mutex::new(()), pending: std::sync::Mutex::new(IrPending::default()) });
    let handle = TrackerHandle { tx, query_tx, sorter: devices.sorter.clone(), ir: ir.clone() };

    let mut out = LiveOutputs {
        app: app.clone(),
        devices,
        store: store::Store::spawn(app.db.clone()),
        report_queue: crate::middleware::report_queue::ReportQueue::new(app.db.clone()),
        chute_tx,
        label_tx,
        led_via_sorter: cfg.sysled.via != "belt",
        jam: JamThrottle::new(cfg.middleware.jam_alert_throttle_ms as i64, cfg.middleware.jam_alert_reset_ms as i64),
        jam_hot: JamHotspot::new(3_600_000, 3, 1_800_000),
        jam_tx,
    };
    let mut machine = Machine::new(cfg, chutes, today, crate::db::now_ms());
    let mut cfg_rx = app.config.subscribe();

    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut publish_at = std::time::Instant::now();
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                ev = rx.recv() => match ev {
                    Some(ev) => {
                        if let DeviceEvent::Sorter { sig, .. } = &ev {
                            ir.on_sorter_signal(sig);
                        }
                        if let DeviceEvent::State { device, connected, detail, ts_ms } = &ev {
                            app.runtime.set_device(*device, *connected, detail.clone(), *ts_ms);
                            let cat = match device { Device::Belt => "belt", Device::Sorter => "sorter", Device::Camera => "camera" };
                            event_log::log(&app.db, if *connected { Level::Info } else { Level::Warn }, cat, if *connected { "connected" } else { "disconnected" }, detail.clone());
                            event_bus::emit("device-state", serde_json::json!({ "device": device, "connected": connected, "ts_ms": ts_ms }));
                        }
                        machine.handle(Input::Device(ev), &mut out);
                    }
                    None => break,
                },
                Some(input) = in_rx.recv() => machine.handle(input, &mut out),
                Some(reply) = query_rx.recv() => {
                    let _ = reply.send(snapshot(&machine));
                }
                _ = cfg_rx.changed() => {
                    let cfg = cfg_rx.borrow().clone();
                    out.led_via_sorter = cfg.sysled.via != "belt";
                    out.jam.set_params(cfg.middleware.jam_alert_throttle_ms as i64, cfg.middleware.jam_alert_reset_ms as i64);
                    machine.set_config(cfg);
                    // 格口表可能一起改了（設定頁存檔後會重載）
                    if let Ok(ch) = load_chutes(&app.db).await { machine.set_chutes(ch); }
                }
                _ = tick.tick() => {
                    machine.handle(Input::Tick { now_ms: crate::db::now_ms() }, &mut out);
                    if publish_at.elapsed() >= Duration::from_millis(500) {
                        publish_at = std::time::Instant::now();
                        event_bus::emit("status", snapshot(&machine));
                    }
                }
            }
        }
        tracing::info!("狀態機結束");
    });

    Ok((handle, TrackerPorts { chute_rx, jam_rx }))
}

fn snapshot(m: &Machine) -> TrackerSnapshot {
    let (head_current, head_queue, head_jam, blocked) = m.head_state();
    TrackerSnapshot {
        belt_running: m.belt_running,
        head_current,
        head_queue,
        head_jam,
        blocked,
        today_count: m.today_count,
        counters: m.counters.clone(),
        current: m.current.and_then(|k| m.parcels.get(&k).cloned()),
        in_flight: m.in_flight().into_iter().cloned().collect(),
        now_ms: crate::db::now_ms(),
    }
}

#[cfg(test)]
mod jam_hotspot_tests {
    use super::JamHotspot;

    #[test]
    fn 同模組一小時內第三次才提示_之後半小時不重複() {
        let mut h = JamHotspot::new(3_600_000, 3, 1_800_000);
        assert_eq!(h.on_jam(22, 0), None); // 模組 3
        assert_eq!(h.on_jam(25, 60_000), None);
        assert_eq!(h.on_jam(21, 120_000), Some((3, 3)));
        assert_eq!(h.on_jam(22, 180_000), None, "剛提示過半小時內不再響");
        assert_eq!(h.on_jam(22, 180_000 + 1_800_000), Some((3, 5)), "半小時後還在卡就再提示，次數累計");
    }

    #[test]
    fn 超過一小時的舊卡件不算_不同模組各自算() {
        let mut h = JamHotspot::new(3_600_000, 3, 1_800_000);
        h.on_jam(2, 0); // 模組 1
        h.on_jam(3, 1_000);
        assert_eq!(h.on_jam(5, 3_700_000), None, "前兩次已超過一小時，只剩這一次");
        h.on_jam(33, 3_700_000); // 模組 4
        h.on_jam(35, 3_700_001);
        assert_eq!(h.on_jam(31, 3_700_002), Some((4, 3)));
    }
}

#[cfg(test)]
mod jam_tests {
    use super::JamThrottle;

    #[test]
    fn 同格口節流_安靜五秒後重置() {
        let mut j = JamThrottle::new(20_000, 5_000);
        assert_eq!(j.on_signal(73, 0), Some(8));
        assert_eq!(j.on_signal(73, 1_000), None, "20 秒內同格口不重報");
        assert_eq!(j.on_signal(25, 1_500), Some(3), "不同格口各自計");
        // 一直有 ~k 進來（堵塞持續，每 2 秒一個）：滿 20 秒才再報
        for t in (3_000..=19_000).step_by(2_000) {
            assert_eq!(j.on_signal(73, t), None, "t={t}");
        }
        assert_eq!(j.on_signal(73, 21_000), Some(8));
        // 清掉後安靜 6 秒再卡：不必等 20 秒，立刻報
        assert_eq!(j.on_signal(73, 27_100), Some(8));
    }
}
