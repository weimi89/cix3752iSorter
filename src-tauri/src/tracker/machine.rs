//! 包裹狀態機本體：純邏輯，所有副作用經 `Outputs` 送出，可用假輸出做單元測試。
//!
//! 流程（單集群）：`~P` 建件 → 相機綁碼 → 非同步問格口 → `~O` 到頭部排隊 → 頭部空著就下 `Kn`
//! → `~c` 得小車 → `~j/~g` 釋放頭部 → `~e` 完成 ／ `~k` 堵塞 ／ `~u` 丟失 ／ `Kx` 取消。
//! 各段時序與規則見 `docs/protocol-spec.md`。

use std::collections::{HashMap, VecDeque};

use crate::config::AppConfig;
use crate::device::{Device, DeviceEvent};
use crate::device::camera::NO_READ;
use crate::protocol::{BeltSignal, Cid, SorterSignal, command};

use super::parcel::{ChuteDecision, ChuteSource, LabelPayload, Parcel, Status};

/// 終態後再留多久才落最終資料並釋放（讓晚到的訊號還能記到事件表）
const FINALIZE_GRACE_MS: i64 = 300;
/// `Kn` 後遲遲沒有 `~c`：超過就取消並釋放頭部，避免整線卡死
const HEAD_STUCK_MS: i64 = 5000;
/// `~k` 之後多久沒有再收到 `~k` 視為堵塞解除
const BLOCK_CLEAR_MS: i64 = 2000;
/// 到頭部排隊卻一直沒輪到／`Kn` 後一直沒終態 → 視為失去追蹤
const STALE_AFTER_O_MS: i64 = 120_000;
/// `~P` 後一直沒有 `~O`（皮帶上消失）
const STALE_AFTER_P_MS: i64 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Led {
    Off,
    Green,
    Red,
    Idle,
}

pub trait Outputs {
    fn belt_cmd(&mut self, cmd: &str);
    fn sorter_cmd(&mut self, cmd: &str);
    fn led_cmd(&mut self, cmd: &str);
    fn store_insert(&mut self, p: &Parcel);
    fn store_update(&mut self, p: &Parcel);
    fn store_event(&mut self, p: &Parcel, ts_ms: i64, source: &'static str, kind: &str, raw: Option<&str>);
    fn store_forget(&mut self, p: &Parcel);
    fn store_daily(&mut self, p: &Parcel);
    /// 向中介機查格口；NoRead 也要送（中介機要拍照存證、計入讀碼失敗統計），但本機已先決定預設口
    fn request_chute(&mut self, p: &Parcel);
    /// 格口決定被接受、且中介機有給面單 → 交給列印
    fn print_label(&mut self, p: &Parcel, printer_port: Option<String>, label: LabelPayload);
    fn parcel_changed(&mut self, p: &Parcel);
    fn log(&mut self, level: crate::event_log::Level, category: &'static str, action: &'static str, msg: String);
    fn jam_alert(&mut self, pos: i32);
    fn chute_decided(&mut self, p: &Parcel);
}

/// 格口對照（來自 `chutes` 表）
#[derive(Clone, Debug)]
pub struct ChuteRow {
    pub code: String,
    pub cid: Cid,
    pub printer_port: Option<String>,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub enum Input {
    Device(DeviceEvent),
    /// 格口解析結果（面單已下載好才算結果；下載失敗的解析器會改回預設口）
    Chute { key: u64, code: String, cid: Cid, source: ChuteSource, response_id: Option<i64>, label: Option<LabelPayload> },
    Tick { now_ms: i64 },
    /// 網頁手動控制皮帶
    BeltStart,
    BeltStop,
    /// 網頁：重置分揀機（等同重連時的處理）
    SorterReset,
    /// 網頁：直接送一條分揀機指令（燈控、光電查詢）
    SorterRaw(String),
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Counters {
    pub today: String,
    pub total: u64,
    pub done: u64,
    pub noread: u64,
    pub defaulted: u64,
    pub abnormal: u64,
}

pub struct Machine {
    pub cfg: AppConfig,
    pub chutes: HashMap<String, ChuteRow>,
    next_key: u64,
    pub parcels: HashMap<u64, Parcel>,
    by_slot: HashMap<u32, u64>,
    by_cart: HashMap<u32, u64>,
    unbound: VecDeque<u64>,
    pending_codes: VecDeque<(String, i64)>,
    head_queue: VecDeque<u64>,
    head_current: Option<u64>,
    head_prev: Option<u64>,
    /// 「堵塞到頭部」而停線中
    head_jam: bool,
    /// 堵塞解除時間點；Some = 堵塞中
    block_until: Option<i64>,
    block_stopped_belt: bool,
    pub belt_running: bool,
    belt_input_bits: u8,
    sorter_input_bits: u8,
    /// 急停按鍵按住中：放開前「急停恢復」不啟動（對應舊系統 `EmergencyStopIng`）
    estop_latched: bool,
    led: Led,
    led_red_until: Option<i64>,
    led_last_activity: i64,
    pub counters: Counters,
    /// 今日件數（含在途）
    pub today_count: u64,
    pub current: Option<u64>,
    now_ms: i64,
}

impl Machine {
    pub fn new(cfg: AppConfig, chutes: HashMap<String, ChuteRow>, today_count: u64, now_ms: i64) -> Self {
        Self {
            cfg,
            chutes,
            next_key: 1,
            parcels: HashMap::new(),
            by_slot: HashMap::new(),
            by_cart: HashMap::new(),
            unbound: VecDeque::new(),
            pending_codes: VecDeque::new(),
            head_queue: VecDeque::new(),
            head_current: None,
            head_prev: None,
            head_jam: false,
            block_until: None,
            block_stopped_belt: false,
            belt_running: false,
            belt_input_bits: 0,
            sorter_input_bits: 0,
            estop_latched: false,
            led: Led::Off,
            led_red_until: None,
            led_last_activity: now_ms,
            counters: Counters { today: today_str(now_ms), ..Default::default() },
            today_count,
            current: None,
            now_ms,
        }
    }

    pub fn set_config(&mut self, cfg: AppConfig) {
        // 急停按鈕的定義改了（換位元、換動作），舊的鎖住狀態就不再有意義
        if cfg.emergency_buttons != self.cfg.emergency_buttons {
            self.estop_latched = false;
        }
        self.cfg = cfg;
    }

    pub fn set_chutes(&mut self, chutes: HashMap<String, ChuteRow>) {
        self.chutes = chutes;
    }

    /// 在途件（給網頁看）
    pub fn in_flight(&self) -> Vec<&Parcel> {
        let mut v: Vec<&Parcel> = self.parcels.values().filter(|p| !p.is_ended()).collect();
        v.sort_by_key(|p| p.p_ms);
        v
    }

    pub fn head_state(&self) -> (Option<u64>, usize, bool, bool) {
        (self.head_current, self.head_queue.len(), self.head_jam, self.block_until.is_some())
    }

    pub fn handle<O: Outputs>(&mut self, input: Input, out: &mut O) {
        match input {
            Input::Device(ev) => self.on_device(ev, out),
            Input::Chute { key, code, cid, source, response_id, label } => self.on_chute(key, code, cid, source, response_id, label, out),
            Input::Tick { now_ms } => self.on_tick(now_ms, out),
            Input::BeltStart => self.belt_start(out, "網頁啟動"),
            Input::BeltStop => self.belt_stop(out, "網頁停止"),
            Input::SorterReset => {
                let now = self.now_ms;
                out.sorter_cmd(command::reset_sorter());
                out.log(crate::event_log::Level::Warn, "sorter", "reset", "網頁重置分揀機".into());
                self.reset_sorter_side(now, out);
            }
            Input::SorterRaw(cmd) => out.sorter_cmd(&cmd),
        }
    }

    // ---------- 裝置事件 ----------

    fn on_device<O: Outputs>(&mut self, ev: DeviceEvent, out: &mut O) {
        match ev {
            DeviceEvent::State { device, connected, ts_ms, .. } => {
                self.now_ms = self.now_ms.max(ts_ms);
                if device == Device::Sorter && connected {
                    // 分揀機重連時已被重置（Kx999;Kk2）：頭部與小車對應全部作廢
                    self.reset_sorter_side(ts_ms, out);
                }
                // 開機警示燈：燈接在哪條線上，就在那條線連上時亮（舊系統同樣做法）
                let led_line = if self.cfg.sysled.via == "belt" { Device::Belt } else { Device::Sorter };
                if device == led_line && connected && self.cfg.sysled.alarm_on_start {
                    self.set_led(Led::Red, ts_ms, out);
                }
                if device == Device::Belt && !connected {
                    self.belt_running = false;
                    // 斷線期間按鈕放開不會再有 ~v 進來：輸入點狀態與急停鎖一併歸零，重連後以新訊號為準
                    self.belt_input_bits = 0;
                    self.estop_latched = false;
                }
            }
            DeviceEvent::Belt { sig, raw, ts_ms } => {
                self.now_ms = self.now_ms.max(ts_ms);
                self.on_belt(sig, &raw, ts_ms, out);
            }
            DeviceEvent::Sorter { sig, raw, ts_ms } => {
                self.now_ms = self.now_ms.max(ts_ms);
                self.on_sorter(sig, &raw, ts_ms, out);
            }
            DeviceEvent::Barcode { code, raw, ts_ms } => {
                self.now_ms = self.now_ms.max(ts_ms);
                self.on_barcode(code, &raw, ts_ms, out);
            }
        }
    }

    fn on_belt<O: Outputs>(&mut self, sig: BeltSignal, raw: &str, ts: i64, out: &mut O) {
        match sig {
            BeltSignal::P { slot, .. } => {
                self.belt_running = true;
                // 同一槽還掛著上一件（沒收到 ~E）：那件已經追不到了
                if let Some(old_key) = self.by_slot.get(&slot).copied() {
                    if let Some(old) = self.parcels.get(&old_key) {
                        if !old.is_ended() && old.o_ms.is_none() {
                            self.end_parcel(old_key, Status::Lost, ts, "slot_reused", out);
                        }
                    }
                }
                let key = self.next_key;
                self.next_key += 1;
                let p = Parcel::new(key, slot, ts);
                self.by_slot.insert(slot, key);
                self.unbound.push_back(key);
                out.store_insert(&p);
                out.store_event(&p, ts, "belt", "P", Some(raw));
                out.parcel_changed(&p);
                self.parcels.insert(key, p);
                self.today_count += 1;
                self.try_bind_pending(key, out);
            }
            BeltSignal::L { slot, len, gap } => {
                self.belt_running = true;
                let Some(key) = self.by_slot.get(&slot).copied() else {
                    out.log(crate::event_log::Level::Warn, "belt", "orphan_L", format!("~L 對應不到包裹: {raw}"));
                    return;
                };
                let too_close = self.cfg.ng.too_close && gap < self.cfg.ng.min_gap;
                let mut ng = false;
                if let Some(p) = self.parcels.get_mut(&key) {
                    p.l_ms = Some(ts);
                    p.ir_length = Some(len);
                    p.gap = Some(gap);
                    out.store_event(p, ts, "belt", "L", Some(raw));
                    if len <= 0 {
                        ng = true;
                    }
                }
                if ng {
                    self.end_parcel(key, Status::TriggerNg, ts, "L_len_zero", out);
                } else {
                    self.led_last_activity = ts;
                    if self.led != Led::Red {
                        self.set_led(Led::Green, ts, out);
                    }
                }
                if too_close {
                    self.belt_stop(out, "間距過近");
                    self.set_led(Led::Red, ts, out);
                }
                if let Some(p) = self.parcels.get(&key) {
                    out.store_update(p);
                    out.parcel_changed(p);
                }
            }
            BeltSignal::O { slot, .. } => {
                self.belt_running = true;
                let Some(key) = self.by_slot.get(&slot).copied() else {
                    out.log(crate::event_log::Level::Warn, "belt", "orphan_O", format!("~O 對應不到包裹: {raw}"));
                    return;
                };
                let mut enqueue = false;
                if let Some(p) = self.parcels.get_mut(&key) {
                    p.o_ms = Some(ts);
                    out.store_event(p, ts, "belt", "O", Some(raw));
                    enqueue = !p.is_ended();
                }
                // 已到交接點的件不再綁碼：之後才到的條碼一定是後面那件的（舊版同樣在 ~O 清掉綁定資格）
                self.unbound.retain(|&k| k != key);
                if enqueue {
                    self.head_queue.push_back(key);
                    self.pump_head(ts, out);
                }
            }
            BeltSignal::E { slot, .. } => {
                self.belt_running = true;
                if let Some(key) = self.by_slot.remove(&slot) {
                    if let Some(p) = self.parcels.get_mut(&key) {
                        p.belt_e_ms = Some(ts);
                        out.store_event(p, ts, "belt", "E", Some(raw));
                    }
                }
            }
            BeltSignal::Stopped => {
                self.belt_running = false;
            }
            BeltSignal::Input { m2, bits } => {
                let prev = self.belt_input_bits;
                self.belt_input_bits = bits;
                self.on_input("belt", m2, prev, bits, out);
            }
            BeltSignal::Unknown(_) => {
                out.log(crate::event_log::Level::Warn, "belt", "unknown", format!("未知訊號: {raw}"));
            }
        }
    }

    fn on_sorter<O: Outputs>(&mut self, sig: SorterSignal, raw: &str, ts: i64, out: &mut O) {
        match sig {
            SorterSignal::C { cart, .. } => {
                let Some(key) = self.head_current else {
                    out.log(crate::event_log::Level::Warn, "sorter", "orphan_c", format!("~c 但頭部沒有待受理的包裹: {raw}"));
                    return;
                };
                // 小車被重用而舊件沒收尾：舊件視為失去追蹤
                if let Some(old_key) = self.by_cart.get(&cart).copied() {
                    if old_key != key {
                        if let Some(old) = self.parcels.get(&old_key) {
                            if !old.is_ended() {
                                self.end_parcel(old_key, Status::Lost, ts, "cart_reused", out);
                            }
                        }
                    }
                }
                self.by_cart.insert(cart, key);
                if let Some(p) = self.parcels.get_mut(&key) {
                    p.cart = Some(cart);
                    p.c_ms = Some(ts);
                    out.store_event(p, ts, "sorter", "c", Some(raw));
                    out.store_update(p);
                    out.parcel_changed(p);
                }
            }
            SorterSignal::J { cart } => {
                let Some(key) = self.by_cart.get(&cart).copied() else { return };
                if let Some(p) = self.parcels.get_mut(&key) {
                    p.j_ms = Some(ts);
                    if p.status == Status::Init {
                        p.status = Status::Received;
                    }
                    out.store_event(p, ts, "sorter", "j", Some(raw));
                    out.store_update(p);
                    out.parcel_changed(p);
                }
                self.release_head_if(key, ts, out);
            }
            SorterSignal::G { cart, .. } => {
                let Some(key) = self.by_cart.get(&cart).copied() else { return };
                if let Some(p) = self.parcels.get_mut(&key) {
                    p.g_ms = Some(ts);
                    out.store_event(p, ts, "sorter", "g", Some(raw));
                }
                self.release_head_if(key, ts, out);
                if self.head_jam {
                    self.head_jam = false;
                    self.belt_start(out, "頭部堵塞解除");
                }
            }
            SorterSignal::N { cart } => {
                if let Some(key) = self.by_cart.get(&cart).copied() {
                    if let Some(p) = self.parcels.get(&key) {
                        out.store_event(p, ts, "sorter", "n", Some(raw));
                    }
                }
            }
            SorterSignal::E { cart } => {
                let Some(key) = self.by_cart.get(&cart).copied() else { return };
                if let Some(p) = self.parcels.get_mut(&key) {
                    p.e_ms = Some(ts);
                    out.store_event(p, ts, "sorter", "e", Some(raw));
                }
                let already = self.parcels.get(&key).is_some_and(|p| p.is_ended());
                if !already {
                    self.end_parcel(key, Status::Done, ts, "e", out);
                }
            }
            SorterSignal::K { cart, pos, .. } => {
                if self.block_until.is_none() {
                    if self.cfg.ng.on_block {
                        self.belt_stop(out, "堵塞");
                        self.block_stopped_belt = true;
                    }
                    if self.cfg.sysled.alarm_on_block {
                        self.set_led(Led::Red, ts, out);
                    }
                }
                // 每個 ~k 都送出去，同格口的節流與「安靜 5 秒後重置」由外層決定（對齊舊 Node）
                out.jam_alert(pos);
                self.block_until = Some(ts + BLOCK_CLEAR_MS);
                let Some(key) = self.by_cart.get(&cart).copied() else { return };
                if let Some(p) = self.parcels.get_mut(&key) {
                    if p.k_ms.is_none() {
                        p.k_ms = Some(ts);
                        p.block_pos = Some(pos);
                        p.was_blocked = true;
                        if !p.status.is_final() {
                            p.status = Status::Blocked;
                        }
                        out.store_event(p, ts, "sorter", "k", Some(raw));
                        out.store_update(p);
                        out.parcel_changed(p);
                    }
                }
            }
            SorterSignal::U { cart, pos, .. } => {
                if self.cfg.ng.on_lost {
                    self.belt_stop(out, "丟失");
                }
                if self.cfg.sysled.alarm_on_lost {
                    self.set_led(Led::Red, ts, out);
                }
                let Some(key) = self.by_cart.get(&cart).copied() else { return };
                let mut status = None;
                if let Some(p) = self.parcels.get_mut(&key) {
                    if p.u_ms.is_none() {
                        p.u_ms = Some(ts);
                        p.lost_pos = Some(pos);
                        out.store_event(p, ts, "sorter", "u", Some(raw));
                        if !p.is_ended() {
                            status = Some(if self.cfg.sorter.u_as_done {
                                Status::Done
                            } else if p.was_blocked {
                                Status::BlockedThenTaken
                            } else {
                                Status::Lost
                            });
                        }
                    }
                }
                if let Some(s) = status {
                    self.end_parcel(key, s, ts, "u", out);
                }
            }
            SorterSignal::X { cart } => {
                if cart >= 0 {
                    if let Some(key) = self.by_cart.get(&(cart as u32)).copied() {
                        if let Some(p) = self.parcels.get(&key) {
                            out.store_event(p, ts, "sorter", "x", Some(raw));
                        }
                    }
                }
            }
            SorterSignal::Q { .. } | SorterSignal::F(_) | SorterSignal::Y(_) => {
                // 診斷類，只留在裝置日誌，不改狀態
            }
            SorterSignal::I { .. } => {
                if self.cfg.ng.on_i {
                    self.belt_stop(out, "~I 觸發");
                    self.set_led(Led::Red, ts, out);
                }
            }
            SorterSignal::Input { m2, bits } => {
                let prev = self.sorter_input_bits;
                self.sorter_input_bits = bits;
                self.on_input("sorter", m2, prev, bits, out);
            }
            SorterSignal::Stopped | SorterSignal::IrStatus(_) | SorterSignal::P1Line(_) => {}
            SorterSignal::Unknown(_) => {
                out.log(crate::event_log::Level::Warn, "sorter", "unknown", format!("未知訊號: {raw}"));
            }
        }
    }

    fn on_barcode<O: Outputs>(&mut self, code: String, raw: &str, ts: i64, out: &mut O) {
        let floor = self.cfg.camera.bind_floor_ms;
        let ceiling = self.cfg.camera.bind_ceiling_ms;
        let expected = self.cfg.camera.bind_expected_ms;
        // 窗口內有多件候選（前一件相機漏拍、還沒過 ~O）時，挑「~P 到條碼」最接近典型延遲的那件；
        // 一律挑最早那件會把後面每一件的條碼都綁到前一件，一路錯到出現空檔為止
        let mut found: Option<(u64, i64)> = None;
        for &key in &self.unbound {
            if let Some(p) = self.parcels.get(&key) {
                let dt = ts - p.p_ms;
                if dt >= floor && dt <= ceiling {
                    let dist = (dt - expected).abs();
                    if found.is_none_or(|(_, best)| dist < best) {
                        found = Some((key, dist));
                    }
                }
            }
        }
        let found = found.map(|(k, _)| k);
        match found {
            Some(key) => {
                self.unbound.retain(|&k| k != key);
                self.bind(key, code, raw, ts, out);
            }
            None => {
                // 條碼比 ~P 早到（floor 為負）：暫存等 P
                if floor < 0 {
                    self.pending_codes.push_back((code, ts));
                } else {
                    out.log(crate::event_log::Level::Warn, "camera", "unmatched", format!("條碼 {code} 沒有可綁定的包裹"));
                }
            }
        }
    }

    fn try_bind_pending<O: Outputs>(&mut self, key: u64, out: &mut O) {
        let floor = self.cfg.camera.bind_floor_ms;
        let ceiling = self.cfg.camera.bind_ceiling_ms;
        let p_ms = match self.parcels.get(&key) {
            Some(p) => p.p_ms,
            None => return,
        };
        let idx = self.pending_codes.iter().position(|(_, cts)| {
            let dt = cts - p_ms;
            dt >= floor && dt <= ceiling
        });
        if let Some(i) = idx {
            let (code, cts) = self.pending_codes.remove(i).unwrap();
            self.unbound.retain(|&k| k != key);
            self.bind(key, code, "(pending)", cts, out);
        }
    }

    fn bind<O: Outputs>(&mut self, key: u64, code: String, raw: &str, ts: i64, out: &mut O) {
        let noread = code == NO_READ;
        if let Some(p) = self.parcels.get_mut(&key) {
            p.barcode = Some(code.clone());
            p.bind_ms = Some(ts);
            out.store_event(p, ts, "camera", "bind", Some(raw));
            out.store_update(p);
            out.parcel_changed(p);
        }
        if noread {
            let (dcode, dcid) = self.default_chute();
            self.on_chute(key, dcode, dcid, ChuteSource::NoRead, None, None, out);
        }
        if let Some(p) = self.parcels.get(&key) {
            out.request_chute(p);
        }
    }

    fn on_chute<O: Outputs>(&mut self, key: u64, code: String, cid: Cid, source: ChuteSource, response_id: Option<i64>, label: Option<LabelPayload>, out: &mut O) {
        let ts = self.now_ms;
        let Some(p) = self.parcels.get_mut(&key) else { return };
        if p.kn_ms.is_some() {
            // 指令已下，回覆太晚：留紀錄但不改決定（老日誌的「返回超時,貨物已下發分揀指令」）
            out.store_event(p, ts, "api", "chute_late", Some(&format!("{code} {cid} {source:?} rid={response_id:?}")));
            out.log(
                crate::event_log::Level::Warn,
                "chute",
                "late",
                format!("條碼 {} 格口 {code} 回覆太晚（{}ms），已用 {}", p.barcode_or_noread(), ts - p.p_ms, p.chute.as_ref().map(|c| c.code.as_str()).unwrap_or("?")),
            );
            return;
        }
        if p.chute.as_ref().is_some_and(|c| c.source != ChuteSource::Pending) && source != ChuteSource::Manual {
            // 已決定（重複回覆）：不改決定也不印
            return;
        }
        p.chute = Some(ChuteDecision { code: code.clone(), cid, source, response_id, decided_ms: ts });
        out.store_event(p, ts, "api", "chute", Some(&format!("{code} {cid} {source:?} rid={response_id:?} label={}", label.is_some())));
        out.store_update(p);
        out.parcel_changed(p);
        out.chute_decided(p);
        if let Some(label) = label {
            let port = self.chutes.get(&code).and_then(|r| r.printer_port.clone());
            let p = &self.parcels[&key];
            out.print_label(p, port, label);
        }
    }

    fn default_chute(&self) -> (String, Cid) {
        let code = self.cfg.general.default_chute.clone();
        match self.chutes.get(&code) {
            Some(row) => (code, row.cid),
            None => (code, Cid(1007301)),
        }
    }

    // ---------- 頭部 ----------

    /// 頭部空著就把下一件下 `Kn`
    fn pump_head<O: Outputs>(&mut self, ts: i64, out: &mut O) {
        if self.head_current.is_some() {
            return;
        }
        while let Some(key) = self.head_queue.pop_front() {
            let Some(p) = self.parcels.get_mut(&key) else { continue };
            if p.is_ended() {
                continue;
            }
            if p.chute.is_none() {
                let (code, cid) = {
                    let code = self.cfg.general.default_chute.clone();
                    let cid = self.chutes.get(&code).map(|r| r.cid).unwrap_or(Cid(1007301));
                    (code, cid)
                };
                let source = if p.barcode.is_none() { ChuteSource::NoRead } else { ChuteSource::Timeout };
                p.chute = Some(ChuteDecision { code, cid, source, response_id: None, decided_ms: ts });
                out.store_event(p, ts, "tracker", "chute_default", Some(&format!("{source:?}")));
            }
            let cid = p.chute.as_ref().map(|c| c.cid).unwrap();
            let cmd = command::kn(cid, self.cfg.sorter.speed, false);
            p.kn_ms = Some(ts);
            out.sorter_cmd(&cmd);
            out.store_event(p, ts, "tracker", "Kn", Some(&cmd));
            out.store_update(p);
            out.parcel_changed(p);
            self.head_current = Some(key);
            self.current = Some(key);
            return;
        }
    }

    fn release_head_if<O: Outputs>(&mut self, key: u64, ts: i64, out: &mut O) {
        if self.head_current == Some(key) {
            self.head_prev = Some(key);
            self.head_current = None;
            self.pump_head(ts, out);
        }
    }

    fn reset_sorter_side<O: Outputs>(&mut self, ts: i64, out: &mut O) {
        let keys: Vec<u64> = self.by_cart.values().copied().collect();
        for key in keys {
            if self.parcels.get(&key).is_some_and(|p| !p.is_ended()) {
                self.end_parcel(key, Status::Lost, ts, "sorter_reset", out);
            }
        }
        self.by_cart.clear();
        if let Some(key) = self.head_current.take() {
            if self.parcels.get(&key).is_some_and(|p| !p.is_ended()) {
                self.end_parcel(key, Status::Lost, ts, "sorter_reset", out);
            }
        }
        self.head_prev = None;
        self.head_jam = false;
        self.block_until = None;
        self.pump_head(ts, out);
    }

    // ---------- 終態 ----------

    fn end_parcel<O: Outputs>(&mut self, key: u64, status: Status, ts: i64, reason: &str, out: &mut O) {
        let Some(p) = self.parcels.get_mut(&key) else { return };
        if p.is_ended() {
            return;
        }
        p.status = status;
        p.ended_ms = Some(ts);
        out.store_event(p, ts, "tracker", "end", Some(&format!("{} ({reason})", status.label())));
        out.store_update(p);
        out.parcel_changed(p);
        if matches!(status, Status::Lost | Status::Cancelled) && reason != "u" {
            out.log(crate::event_log::Level::Warn, "tracker", "end", format!("包裹 {} → {}（{reason}）", p.barcode_or_noread(), status.label()));
        }
        self.unbound.retain(|&k| k != key);
        if self.head_current == Some(key) {
            self.head_current = None;
            self.head_prev = Some(key);
            self.pump_head(ts, out);
        }
    }

    // ---------- 皮帶 / 燈 ----------

    // 皮帶運轉中沒有心跳訊號（只有停止時週期送 `~k-1`），所以送出啟停指令時先樂觀更新
    // `belt_running`，畫面上的單顆啟停鈕才會立刻切換；之後 `~k-1`／`~P` 會把它糾正回實況
    fn belt_stop<O: Outputs>(&mut self, out: &mut O, reason: &str) {
        out.belt_cmd(&self.cfg.belt.cmd.stop.clone());
        self.belt_running = false;
        out.log(crate::event_log::Level::Warn, "belt", "stop", format!("停線：{reason}"));
    }

    fn belt_start<O: Outputs>(&mut self, out: &mut O, reason: &str) {
        let run = self.cfg.belt.cmd.run.clone();
        let auto = self.cfg.belt.cmd.auto.clone();
        out.belt_cmd(&run);
        if !self.cfg.belt.default_run && auto != run {
            out.belt_cmd(&auto);
        }
        self.belt_running = true;
        out.log(crate::event_log::Level::Info, "belt", "start", format!("啟動：{reason}"));
    }

    fn set_led<O: Outputs>(&mut self, led: Led, ts: i64, out: &mut O) {
        if led == Led::Red {
            self.led_red_until = Some(ts + self.cfg.sysled.alarm_hold_ms as i64);
        }
        if self.led == led {
            return;
        }
        self.led = led;
        let cmd = match led {
            Led::Off => self.cfg.sysled.cmd.off.clone(),
            Led::Green => self.cfg.sysled.cmd.green.clone(),
            Led::Red => self.cfg.sysled.cmd.red.clone(),
            Led::Idle => self.cfg.sysled.cmd.idle.clone(),
        };
        out.led_cmd(&cmd);
    }

    fn on_input<O: Outputs>(&mut self, device: &str, m2: u32, prev: u8, bits: u8, out: &mut O) {
        let buttons = self.cfg.emergency_buttons.clone();
        for b in buttons.iter().filter(|b| b.device == device && b.m2 == m2 && b.bit < 8) {
            let mask = 0x80u8 >> b.bit;
            let rising = prev & mask == 0 && bits & mask != 0;
            let falling = prev & mask != 0 && bits & mask == 0;
            match b.action.as_str() {
                "stop" if rising => self.belt_stop(out, &format!("按鈕「{}」", b.describe)),
                "start" if rising => self.belt_start(out, &format!("按鈕「{}」", b.describe)),
                // 急停：按下停線並鎖住，放開才解鎖；鎖住期間「急停恢復」按了也不動（舊系統同樣互鎖）
                "estop" if rising => {
                    self.estop_latched = true;
                    self.belt_stop(out, &format!("急停「{}」", b.describe));
                }
                "estop" if falling => {
                    self.estop_latched = false;
                    out.log(crate::event_log::Level::Info, "belt", "estop_release", format!("急停「{}」已放開", b.describe));
                }
                "estop_release" if rising => {
                    if self.estop_latched {
                        out.log(crate::event_log::Level::Warn, "belt", "estop_blocked", format!("急停仍按著，「{}」不啟動", b.describe));
                    } else {
                        self.belt_start(out, &format!("按鈕「{}」", b.describe));
                    }
                }
                _ => {}
            }
        }
    }

    // ---------- 定時 ----------

    fn on_tick<O: Outputs>(&mut self, now: i64, out: &mut O) {
        self.now_ms = self.now_ms.max(now);
        let now = self.now_ms;

        // 頭部：Kn 後沒 ~c
        if let Some(key) = self.head_current {
            let (kn, c, j, g, belt_e, stop_sent, cart, kx) = match self.parcels.get(&key) {
                Some(p) => (p.kn_ms, p.c_ms, p.j_ms, p.g_ms, p.belt_e_ms, p.belt_stop_sent, p.cart, p.kx_ms),
                None => (None, None, None, None, None, false, None, None),
            };
            if let Some(kn) = kn {
                if c.is_none() {
                    if !stop_sent && now - kn > self.cfg.sorter.c_timeout_ms as i64 {
                        if let Some(p) = self.parcels.get_mut(&key) {
                            p.belt_stop_sent = true;
                        }
                        self.belt_stop(out, "Kn 後收不到 ~c");
                    }
                    if now - kn > HEAD_STUCK_MS {
                        self.end_parcel(key, Status::Cancelled, now, "no_c", out);
                    }
                } else if j.is_none() && g.is_none() && kx.is_none() {
                    // 收到 ~c 但一直沒上車，而皮帶已把它送出去 → 取消
                    if let Some(e) = belt_e {
                        if now - e > self.cfg.sorter.kx_after_e_ms as i64 {
                            if let Some(cart) = cart {
                                out.sorter_cmd(&command::kx(cart));
                            }
                            if let Some(p) = self.parcels.get_mut(&key) {
                                p.kx_ms = Some(now);
                                out.store_event(p, now, "tracker", "Kx", Some("rec ~c but not ~j"));
                            }
                            if self.cfg.ng.on_cancel {
                                self.belt_stop(out, "指令取消");
                            }
                            if self.cfg.sysled.alarm_on_cancel {
                                self.set_led(Led::Red, now, out);
                            }
                            self.end_parcel(key, Status::Cancelled, now, "kx", out);
                        }
                    }
                }
            }
        }

        // 堵塞到頭部：堵塞期間上一件上車了卻一直沒離開頭部（只在堵塞中判斷；
        // 平時 ~j→~g 本來就有兩成會超過 550ms，不設這個條件會一直誤停線）
        if !self.head_jam && self.block_until.is_some() {
            if let Some(prev) = self.head_prev.and_then(|k| self.parcels.get(&k)) {
                if let (Some(j), None) = (prev.j_ms, prev.g_ms) {
                    if !prev.is_ended() && now - j > (55_000 / self.cfg.sorter.speed.max(1)) as i64 {
                        self.head_jam = true;
                        self.belt_stop(out, "堵塞到分揀機頭部");
                    }
                }
            }
        }

        // 堵塞解除
        if let Some(until) = self.block_until {
            if now >= until {
                self.block_until = None;
                if self.block_stopped_belt || self.head_jam {
                    self.block_stopped_belt = false;
                    self.head_jam = false;
                    self.belt_start(out, "堵塞解除");
                }
            }
        }

        // 暫存條碼過期
        let floor = self.cfg.camera.bind_floor_ms;
        while let Some((_, cts)) = self.pending_codes.front() {
            if now - cts > -floor.min(0) + 50 {
                let (code, _) = self.pending_codes.pop_front().unwrap();
                out.log(crate::event_log::Level::Warn, "camera", "unmatched", format!("條碼 {code} 沒有可綁定的包裹"));
            } else {
                break;
            }
        }
        // 綁碼窗口過了還沒條碼 → 不再等
        let ceiling = self.cfg.camera.bind_ceiling_ms;
        self.unbound.retain(|k| self.parcels.get(k).is_some_and(|p| now - p.p_ms <= ceiling));

        // 失去追蹤的兜底：避免記憶體與在途件清單無限累積
        let stale: Vec<(u64, &'static str)> = self
            .parcels
            .values()
            .filter(|p| !p.is_ended())
            .filter_map(|p| {
                if p.o_ms.is_none() && now - p.p_ms > STALE_AFTER_P_MS {
                    Some((p.key, "stale_no_O"))
                } else if p.o_ms.is_some_and(|o| now - o > STALE_AFTER_O_MS) {
                    Some((p.key, "stale_after_O"))
                } else {
                    None
                }
            })
            .collect();
        for (key, reason) in stale {
            self.head_queue.retain(|&k| k != key);
            self.end_parcel(key, Status::Lost, now, reason, out);
        }

        // 終態收尾
        let done: Vec<u64> = self
            .parcels
            .values()
            .filter(|p| p.ended_ms.is_some_and(|e| now - e >= FINALIZE_GRACE_MS))
            .map(|p| p.key)
            .collect();
        for key in done {
            if let Some(p) = self.parcels.remove(&key) {
                out.store_update(&p);
                out.store_daily(&p);
                out.store_forget(&p);
                self.account(&p);
                if let Some(cart) = p.cart {
                    if self.by_cart.get(&cart) == Some(&key) {
                        self.by_cart.remove(&cart);
                    }
                }
                if self.by_slot.get(&p.slot) == Some(&key) {
                    self.by_slot.remove(&p.slot);
                }
                if self.head_prev == Some(key) {
                    self.head_prev = None;
                }
                if self.current == Some(key) {
                    self.current = None;
                }
            }
        }

        // 燈號
        if let Some(until) = self.led_red_until {
            if now >= until {
                self.led_red_until = None;
                self.set_led(Led::Green, now, out);
            }
        }
        if self.led == Led::Green && now - self.led_last_activity > self.cfg.sysled.idle_after_ms as i64 {
            self.set_led(Led::Idle, now, out);
        }

        // 跨日
        let today = today_str(now);
        if self.counters.today != today {
            self.counters = Counters { today, ..Default::default() };
            self.today_count = 0;
        }
    }

    fn account(&mut self, p: &Parcel) {
        self.counters.total += 1;
        if p.status == Status::Done {
            self.counters.done += 1;
        } else {
            self.counters.abnormal += 1;
        }
        if p.barcode.is_none() || p.barcode.as_deref() == Some(NO_READ) {
            self.counters.noread += 1;
        }
        if p.chute.as_ref().is_some_and(|c| c.source != ChuteSource::Api && c.source != ChuteSource::Manual) {
            self.counters.defaulted += 1;
        }
    }
}

fn today_str(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}
