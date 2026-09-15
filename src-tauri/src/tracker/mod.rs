//! 包裹狀態機 task：單一 owner，裝置事件、格口結果、定時 tick、網頁指令全部匯進來依序處理。

pub mod machine;
pub mod parcel;
pub mod store;

#[cfg(test)]
mod machine_tests;

use std::collections::HashMap;
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
}

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
    pub fn chute_result(&self, key: u64, code: String, cid: Cid, source: ChuteSource, response_id: Option<i64>, label: Option<LabelPayload>) {
        // 送不進去只會讓那件在 ~O 時走預設口，但要留下痕跡，現場才查得到「面單其實抓到了」
        if self.tx.try_send(Input::Chute { key, code: code.clone(), cid, source, response_id, label }).is_err() {
            tracing::error!(key, %code, "狀態機佇列滿，格口結果丟棄（該件將走預設口）");
        }
    }
    pub async fn snapshot(&self) -> Option<TrackerSnapshot> {
        let (tx, rx) = oneshot::channel();
        self.query_tx.send(tx).await.ok()?;
        rx.await.ok()
    }
}

pub async fn load_chutes(db: &crate::db::DbPool) -> anyhow::Result<HashMap<String, ChuteRow>> {
    let rows: Vec<(String, i64, Option<String>, i64)> =
        sqlx::query_as("SELECT code, cid, printer_port, enabled FROM chutes").fetch_all(db).await?;
    Ok(rows
        .into_iter()
        .map(|(code, cid, printer_port, enabled)| {
            (code.clone(), ChuteRow { code, cid: Cid(cid as u32), printer_port, enabled: enabled != 0 })
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
    jam_last: HashMap<i32, i64>,
    jam_throttle_ms: i64,
    jam_tx: mpsc::Sender<i32>,
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
        let now = crate::db::now_ms();
        let chute_no = pos / 10 + 1;
        let last = self.jam_last.get(&chute_no).copied().unwrap_or(0);
        if now - last < self.jam_throttle_ms {
            return;
        }
        self.jam_last.insert(chute_no, now);
        let _ = self.jam_tx.try_send(chute_no);
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
    /// 卡件告警（格口號）
    pub jam_rx: mpsc::Receiver<i32>,
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
    let (jam_tx, jam_rx) = mpsc::channel::<i32>(64);
    let handle = TrackerHandle { tx, query_tx };

    let mut out = LiveOutputs {
        app: app.clone(),
        devices,
        store: store::Store::spawn(app.db.clone()),
        report_queue: crate::middleware::report_queue::ReportQueue::new(app.db.clone()),
        chute_tx,
        label_tx,
        led_via_sorter: cfg.sysled.via != "belt",
        jam_last: HashMap::new(),
        jam_throttle_ms: cfg.middleware.jam_alert_throttle_ms as i64,
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
                    out.jam_throttle_ms = cfg.middleware.jam_alert_throttle_ms as i64;
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
    }
}
