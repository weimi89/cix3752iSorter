//! 狀態機單元測試：用假輸出把副作用收集起來逐一斷言。時序用 `docs/protocol-spec.md` 的實測中位數。

use std::collections::HashMap;

use super::machine::*;
use super::parcel::{ChuteSource, Status};
use crate::config::AppConfig;
use crate::device::{Device, DeviceEvent};
use crate::protocol::{Cid, parse_belt, parse_sorter};

#[derive(Default)]
struct Fake {
    belt: Vec<String>,
    sorter: Vec<String>,
    led: Vec<String>,
    inserted: Vec<String>,
    events: Vec<(String, String)>,
    chute_requests: Vec<(u64, String)>,
    logs: Vec<String>,
    jams: Vec<i32>,
    decided: Vec<(String, String)>,
    forgotten: Vec<String>,
    printed: Vec<(String, String, Option<String>)>,
    /// (cart, pos, 有沒有對到包裹)
    jam_records: Vec<(u32, i32, bool)>,
    bag_ops: Vec<super::machine::BagOp>,
    /// (code, seq, count, limit)
    bag_full: Vec<(String, u32, u32, u32)>,
}

impl Outputs for Fake {
    fn belt_cmd(&mut self, cmd: &str) {
        self.belt.push(cmd.to_string());
    }
    fn sorter_cmd(&mut self, cmd: &str) {
        self.sorter.push(cmd.to_string());
    }
    fn led_cmd(&mut self, cmd: &str) {
        self.led.push(cmd.to_string());
    }
    fn store_insert(&mut self, p: &super::parcel::Parcel) {
        self.inserted.push(p.ulid.clone());
    }
    fn store_update(&mut self, _p: &super::parcel::Parcel) {}
    fn store_event(&mut self, p: &super::parcel::Parcel, _ts: i64, _s: &'static str, kind: &str, _raw: Option<&str>) {
        self.events.push((p.ulid.clone(), kind.to_string()));
    }
    fn store_forget(&mut self, p: &super::parcel::Parcel) {
        self.forgotten.push(p.ulid.clone());
    }
    fn store_daily(&mut self, _p: &super::parcel::Parcel) {}
    fn request_chute(&mut self, p: &super::parcel::Parcel) {
        self.chute_requests.push((p.key, p.barcode_or_noread().to_string()));
    }
    fn parcel_changed(&mut self, _p: &super::parcel::Parcel) {}
    fn log(&mut self, _l: crate::event_log::Level, _c: &'static str, action: &'static str, msg: String) {
        self.logs.push(format!("{action}: {msg}"));
    }
    fn jam_alert(&mut self, pos: i32) {
        self.jams.push(pos);
    }
    fn chute_decided(&mut self, p: &super::parcel::Parcel) {
        let c = p.chute.as_ref().unwrap();
        self.decided.push((p.barcode_or_noread().to_string(), c.code.clone()));
    }
    fn print_label(&mut self, p: &super::parcel::Parcel, printer_port: Option<String>, _label: super::parcel::LabelPayload) {
        self.printed.push((p.barcode_or_noread().to_string(), p.chute.as_ref().unwrap().code.clone(), printer_port));
    }
    fn store_jam(&mut self, _ts: i64, cart: u32, pos: i32, parcel: Option<&super::parcel::Parcel>) {
        self.jam_records.push((cart, pos, parcel.is_some()));
    }
    fn store_bag(&mut self, op: super::machine::BagOp) {
        self.bag_ops.push(op);
    }
    fn bag_full(&mut self, code: &str, seq: u32, count: u32, limit: u32) {
        self.bag_full.push((code.to_string(), seq, count, limit));
    }
}

fn chutes() -> HashMap<String, ChuteRow> {
    let mut m = HashMap::new();
    for (code, cid, port) in [("L1", 1000323, Some("1-8.1")), ("R3", 1003324, None), ("RS", 1007301, None), ("LS", 1007323, None)] {
        m.insert(code.to_string(), ChuteRow { code: code.into(), cid: Cid(cid), printer_port: port.map(String::from), enabled: true, label: code.into(), sort_order: 0, bag_limit: 0 });
    }
    m
}

struct Sim {
    m: Machine,
    out: Fake,
    now: i64,
}

impl Sim {
    fn new() -> Self {
        let now = 1_789_000_000_000;
        Self { m: Machine::new(AppConfig::default(), chutes(), 0, now), out: Fake::default(), now }
    }

    fn at(&mut self, dt: i64) -> &mut Self {
        self.now += dt;
        self.m.handle(Input::Tick { now_ms: self.now }, &mut self.out);
        self
    }

    fn belt(&mut self, line: &str) -> &mut Self {
        let sig = parse_belt(line).unwrap();
        self.m.handle(Input::Device(DeviceEvent::Belt { sig, raw: line.into(), ts_ms: self.now }), &mut self.out);
        self
    }

    fn sorter(&mut self, line: &str) -> &mut Self {
        let sig = parse_sorter(line).unwrap();
        self.m.handle(Input::Device(DeviceEvent::Sorter { sig, raw: line.into(), ts_ms: self.now }), &mut self.out);
        self
    }

    fn barcode(&mut self, code: &str) -> &mut Self {
        self.m.handle(Input::Device(DeviceEvent::Barcode { code: code.into(), raw: code.into(), ts_ms: self.now }), &mut self.out);
        self
    }

    fn chute(&mut self, key: u64, code: &str) -> &mut Self {
        let cid = chutes()[code].cid;
        self.m.handle(Input::Chute { key, code: code.into(), cid, source: ChuteSource::Api, response_id: Some(99), reason: None, label: None }, &mut self.out);
        self
    }

    /// 帶面單的格口結果
    fn chute_with_label(&mut self, key: u64, code: &str) -> &mut Self {
        let cid = chutes()[code].cid;
        let label = super::parcel::LabelPayload { bytes: vec![1, 2, 3], print_profile: Some("PAPER-01#100*150".into()), is_error_label: false };
        self.m.handle(Input::Chute { key, code: code.into(), cid, source: ChuteSource::Api, response_id: Some(99), reason: None, label: Some(label) }, &mut self.out);
        self
    }

    fn parcel(&self, key: u64) -> &super::parcel::Parcel {
        self.m.parcels.get(&key).expect("包裹應還在記憶體")
    }
}

#[test]
fn 正常件_p_綁碼_問格口_o_kn_c_j_g_e_完成() {
    let mut s = Sim::new();
    s.belt("~P24 1");
    assert_eq!(s.out.inserted.len(), 1, "~P 立即落 DB");
    s.at(226).barcode("99K00064643");
    assert_eq!(s.out.chute_requests, vec![(1, "99K00064643".to_string())]);
    s.at(357).chute(1, "R3");
    assert_eq!(s.out.decided, vec![("99K00064643".to_string(), "R3".to_string())]);
    s.at(140).belt("~L24 25 111");
    s.at(950).belt("~O24 1");
    assert_eq!(s.out.sorter, vec!["Kn 101 01 33 100 24 1"], "~O 時頭部空著就立刻下 Kn");
    s.at(14).sorter("~c7 7 410 1");
    assert_eq!(s.parcel(1).cart, Some(7));
    s.at(12).sorter("~j7 1");
    assert_eq!(s.parcel(1).status, Status::Received);
    assert_eq!(s.m.head_state().0, None, "~j 後頭部釋放");
    s.at(426).sorter("~g7 -1 1");
    s.at(400).belt("~E24 1");
    s.at(700).sorter("~e7");
    assert_eq!(s.parcel(1).status, Status::Done);
    assert!(s.parcel(1).is_ended());
    s.at(400);
    assert!(s.m.parcels.get(&1).is_none(), "終態 300ms 後釋放記憶體");
    assert_eq!(s.out.forgotten.len(), 1);
    assert_eq!(s.m.counters.done, 1);
    assert!(s.out.belt.is_empty(), "正常件不該動皮帶");
}

#[test]
fn 頭部忙碌時第二件排隊_釋放後才下kn() {
    let mut s = Sim::new();
    s.belt("~P1 1").at(200).barcode("A0000000001").chute(1, "L1");
    s.at(500).belt("~P2 1").at(200).barcode("A0000000002").chute(2, "R3");
    s.at(600).belt("~O1 1");
    assert_eq!(s.out.sorter.len(), 1);
    s.at(700).belt("~O2 1");
    assert_eq!(s.out.sorter.len(), 1, "頭部還有未受理的件，第二件等");
    s.at(14).sorter("~c3 3 410 1").at(12).sorter("~j3 1");
    assert_eq!(s.out.sorter.len(), 2, "~j 釋放後第二件立刻下 Kn");
    assert_eq!(s.out.sorter[1], "Kn 101 01 33 100 24 1");
}

#[test]
fn 格口回覆太晚_走預設口_回覆只記錄() {
    let mut s = Sim::new();
    s.belt("~P5 1").at(226).barcode("B0000000001");
    s.at(1100).belt("~O5 1");
    assert_eq!(s.out.sorter, vec!["Kn 101 01 73 100 1 1"], "預設口 RS=1007301");
    assert_eq!(s.parcel(1).chute.as_ref().unwrap().source, ChuteSource::Timeout);
    assert_eq!(s.parcel(1).chute.as_ref().unwrap().reason.as_deref(), Some("TIMEOUT"), "沒回覆就下指令 → 原因是逾時");
    s.at(50).chute(1, "L1");
    assert_eq!(s.parcel(1).chute.as_ref().unwrap().code, "RS", "已下指令，不改");
    assert_eq!(s.parcel(1).chute.as_ref().unwrap().reason.as_deref(), Some("LATE"), "回覆到了但太晚 → 原因改成回覆太晚，統計才分得出中介機是沒回還是回太慢");
    assert!(s.out.logs.iter().any(|l| l.starts_with("late:")));
    assert!(s.out.events.iter().any(|(_, k)| k == "chute_late"));
}

#[test]
fn noread_本機立刻走預設口_仍通知中介機存證() {
    let mut s = Sim::new();
    s.belt("~P5 1").at(226).barcode("NoRead");
    // 中介機要拍照與計數，所以照樣送請求
    assert_eq!(s.out.chute_requests, vec![(1, "NoRead".to_string())]);
    let c = s.parcel(1).chute.as_ref().unwrap();
    assert_eq!((c.code.as_str(), c.source, c.reason.as_deref()), ("RS", ChuteSource::NoRead, Some("NOREAD")));
    // 中介機就算回了東西（含面單）也不改決定、不印
    s.chute_with_label(1, "L1");
    assert_eq!(s.parcel(1).chute.as_ref().unwrap().code, "RS");
    assert!(s.out.printed.is_empty());
}

#[test]
fn 格口被接受才印面單_回覆太晚不印() {
    let mut s = Sim::new();
    s.belt("~P5 1").at(226).barcode("A1").chute_with_label(1, "L1");
    assert_eq!(s.out.printed, vec![("A1".to_string(), "L1".to_string(), Some("1-8.1".to_string()))]);

    // 第二件：~O 前沒答案 → 預設口下 Kn，之後面單才到 → 不印
    s.belt("~P6 1").at(226).barcode("A2").at(1300).belt("~O6 1");
    assert!(s.parcel(2).kn_ms.is_some());
    s.chute_with_label(2, "L1");
    assert_eq!(s.out.printed.len(), 1);
    assert_eq!(s.parcel(2).chute.as_ref().unwrap().code, "RS");
}

#[test]
fn 條碼早於p到達_暫存後綁定() {
    let mut s = Sim::new();
    s.barcode("C0000000001");
    assert!(s.out.chute_requests.is_empty());
    s.at(50).belt("~P9 1");
    assert_eq!(s.out.chute_requests, vec![(1, "C0000000001".to_string())]);
    assert_eq!(s.parcel(1).barcode.as_deref(), Some("C0000000001"));
}

#[test]
fn 條碼超出窗口不綁_下一件才綁() {
    let mut s = Sim::new();
    s.belt("~P1 1");
    s.at(2500).belt("~P2 1");
    s.at(200).barcode("D0000000001");
    assert_eq!(s.parcel(1).barcode, None, "第一件已超過 2000ms 窗口");
    assert_eq!(s.parcel(2).barcode.as_deref(), Some("D0000000001"));
}

#[test]
fn 長度為零_觸發異常_不進頭部() {
    let mut s = Sim::new();
    s.belt("~P3 1").at(300).belt("~L3 0 40");
    assert_eq!(s.parcel(1).status, Status::TriggerNg);
    s.at(1000).belt("~O3 1");
    assert!(s.out.sorter.is_empty(), "異常件不下 Kn");
}

#[test]
fn 堵塞_停線_紅燈_告警_兩秒後恢復() {
    let mut s = Sim::new();
    s.belt("~P1 1").at(200).barcode("E0000000001").chute(1, "L1");
    s.at(1100).belt("~O1 1").at(14).sorter("~c2 2 410 1").at(12).sorter("~j2 1").at(400).sorter("~g2 -1 1");
    s.at(3000).sorter("~k2 73 73");
    assert_eq!(s.out.belt, vec!["KM998 1"], "ng.on_block 預設開 → 停線");
    assert_eq!(s.out.jams, vec![73]);
    assert!(s.out.led.contains(&"KL 0 2 3200".to_string()));
    assert_eq!(s.parcel(1).status, Status::Blocked);
    s.at(500).sorter("~k2 73 73");
    assert_eq!(s.out.jams, vec![73, 73], "每個 ~k 都交給外層節流（JamThrottle 另有測試）");
    assert_eq!(s.out.jam_records, vec![(2, 73, true)], "同一件持續堵塞只記一筆卡件事件");
    s.at(1800);
    assert_eq!(s.out.belt.len(), 1, "距最後一次 ~k 未滿 2 秒");
    s.at(300);
    assert_eq!(s.out.belt, vec!["KM998 1", "KM998 3"], "2 秒沒再收到 ~k → 重新啟動");
    // 之後被取走
    s.at(1000).sorter("~u2 73 -999");
    assert_eq!(s.parcel(1).status, Status::BlockedThenTaken);
}

#[test]
fn 丟失_未堵塞為失去追蹤() {
    let mut s = Sim::new();
    s.belt("~P1 1").at(200).barcode("F0000000001").chute(1, "L1");
    s.at(1100).belt("~O1 1").at(14).sorter("~c2 2 410 1").at(12).sorter("~j2 1").at(400).sorter("~g2 -1 1");
    s.at(3000).sorter("~u2 13 5");
    assert_eq!(s.parcel(1).status, Status::Lost);
    assert!(s.out.belt.is_empty(), "ng.on_lost 預設關");
}

#[test]
fn 收到c沒收到j_皮帶已e_發kx取消() {
    let mut s = Sim::new();
    s.belt("~P1 1").at(200).barcode("G0000000001").chute(1, "L1");
    s.at(1100).belt("~O1 1").at(14).sorter("~c4 4 410 1");
    s.at(300).belt("~E1 1");
    s.at(50);
    assert_eq!(s.out.sorter.len(), 1, "~E 後 100ms 內還在等");
    s.at(80);
    assert_eq!(s.out.sorter, vec!["Kn 101 01 03 100 23 1", "Kx 4"], "位置 0 側 3 → 「03」（與舊程式 %d%d 一致）");
    assert_eq!(s.parcel(1).status, Status::Cancelled);
    assert_eq!(s.m.head_state().0, None, "取消後頭部釋放");
}

#[test]
fn kn後收不到c_停線_五秒後取消釋放頭部() {
    let mut s = Sim::new();
    s.belt("~P1 1").at(200).barcode("H0000000001").chute(1, "L1");
    s.at(1100).belt("~O1 1");
    s.at(350);
    assert_eq!(s.out.belt, vec!["KM998 1"], "300ms 沒 ~c 停線");
    s.at(4700);
    assert_eq!(s.parcel(1).status, Status::Cancelled);
    assert_eq!(s.m.head_state().0, None);
}

#[test]
fn 平時j到g超過550ms_不停線() {
    let mut s = Sim::new();
    s.belt("~P1 1").at(200).barcode("I0000000001").chute(1, "L1");
    s.at(1100).belt("~O1 1").at(14).sorter("~c5 5 410 1").at(12).sorter("~j5 1");
    s.at(800).sorter("~g5 -1 1");
    assert!(s.out.belt.is_empty(), "沒有堵塞時 ~j→~g 慢一點是正常的");
}

#[test]
fn 堵塞中上一件有j沒g_視為堵到頭部_解除後啟動() {
    let mut s = Sim::new();
    let mut cfg = AppConfig::default();
    cfg.ng.on_block = false; // 關掉堵塞停線，單獨看頭部規則
    s.m.set_config(cfg);
    s.belt("~P1 1").at(200).barcode("I0000000001").chute(1, "L1");
    s.at(1100).belt("~O1 1").at(14).sorter("~c5 5 410 1").at(12).sorter("~j5 1");
    s.at(100).sorter("~k9 42 42");
    assert!(s.out.belt.is_empty(), "堵塞停線已關");
    s.at(500);
    assert_eq!(s.out.belt, vec!["KM998 1"], "堵塞中且上一件 550ms 沒 ~g → 停線");
    s.at(2100);
    assert_eq!(s.out.belt, vec!["KM998 1", "KM998 3"], "堵塞解除後啟動");
}

#[test]
fn 分揀機重連_在途件全部失去追蹤_頭部清空() {
    let mut s = Sim::new();
    s.belt("~P1 1").at(200).barcode("J0000000001").chute(1, "L1");
    s.at(1100).belt("~O1 1").at(14).sorter("~c5 5 410 1");
    s.m.handle(
        Input::Device(DeviceEvent::State { device: Device::Sorter, connected: true, detail: "x".into(), ts_ms: s.now }),
        &mut s.out,
    );
    assert_eq!(s.parcel(1).status, Status::Lost);
    assert_eq!(s.m.head_state().0, None);
}

#[test]
fn 急停按鈕_上升緣觸發() {
    let mut s = Sim::new();
    let mut cfg = AppConfig::default();
    cfg.emergency_buttons.push(crate::config::EmergencyButton {
        describe: "急停".into(),
        device: "belt".into(),
        m2: 0,
        bit: 2,
        action: "stop".into(),
    });
    s.m.set_config(cfg);
    s.belt("~v0 0");
    s.belt("~v0 32");
    assert_eq!(s.out.belt, vec!["KM998 1"], "bit2 = 0b00100000 = 32 上升緣");
    s.belt("~v0 32");
    assert_eq!(s.out.belt.len(), 1, "維持高電位不重複觸發");
}

#[test]
fn 在途件久未有o_兜底為失去追蹤_不累積記憶體() {
    let mut s = Sim::new();
    s.belt("~P1 1");
    s.at(61_000);
    assert_eq!(s.parcel(1).status, Status::Lost);
    s.at(400);
    assert!(s.m.parcels.is_empty());
}

#[test]
fn 燈號_作業綠_閒置十秒轉閒置燈() {
    let mut s = Sim::new();
    s.belt("~P1 1").at(300).belt("~L1 20 50");
    assert_eq!(s.out.led, vec!["KL 0 2 3010"]);
    s.at(10_100);
    assert_eq!(s.out.led, vec!["KL 0 2 3010", "KL 0 2 3001"]);
}

#[test]
fn 網頁啟停皮帶_立即反映運轉狀態() {
    let mut s = Sim::new();
    assert!(!s.m.belt_running);
    s.m.handle(Input::BeltStart, &mut s.out);
    assert!(s.m.belt_running, "啟動後畫面要能立刻切成「停止」鈕");
    assert_eq!(s.out.belt.last().map(String::as_str), Some("KM998 3"));
    s.m.handle(Input::BeltStop, &mut s.out);
    assert!(!s.m.belt_running);
    assert_eq!(s.out.belt.last().map(String::as_str), Some("KM998 1"));
    // 皮帶回報停止中（~k-1）一樣會把狀態拉回 false
    s.m.handle(Input::BeltStart, &mut s.out);
    s.belt("~k-1 -1");
    assert!(!s.m.belt_running);
}

#[test]
fn 綁碼_窗口內多件候選_挑最接近典型延遲的() {
    let mut s = Sim::new();
    // 第一件相機漏拍；第二件在 400ms 後上線，它的條碼在自己 ~P 後 226ms 到
    s.belt("~P1 1").at(400).belt("~P2 1").at(226).barcode("B0000000002");
    assert_eq!(s.parcel(1).barcode, None, "第一件不該搶到第二件的條碼");
    assert_eq!(s.parcel(2).barcode.as_deref(), Some("B0000000002"));
}

#[test]
fn 綁碼_過了交接點就不再綁() {
    let mut s = Sim::new();
    s.belt("~P1 1").at(1300).belt("~O1 1").at(100).barcode("B0000000001");
    assert_eq!(s.parcel(1).barcode, None, "~O 後才到的條碼不綁這件");
    // 沒人可綁的條碼先暫存等下一個 ~P，暫存期過了才記「沒有可綁定的包裹」
    s.at(200);
    assert_eq!(s.parcel(1).barcode, None);
    assert!(s.out.logs.iter().any(|l| l.contains("沒有可綁定的包裹")), "{:?}", s.out.logs);
}

#[test]
fn 急停互鎖_按住時恢復無效_放開後才能啟動() {
    let mut s = Sim::new();
    let mut cfg = AppConfig::default();
    cfg.emergency_buttons = vec![
        crate::config::EmergencyButton { describe: "急停".into(), device: "belt".into(), m2: 0, bit: 2, action: "estop".into() },
        crate::config::EmergencyButton { describe: "恢復".into(), device: "belt".into(), m2: 0, bit: 3, action: "estop_release".into() },
    ];
    s.m.set_config(cfg);
    s.belt("~v0 32"); // bit2 = 0b00100000 → 急停按下
    assert_eq!(s.out.belt, vec!["KM998 1"]);
    s.belt("~v0 48"); // 急停仍按著（32）+ 恢復按下（16）→ 不啟動
    assert_eq!(s.out.belt, vec!["KM998 1"]);
    s.belt("~v0 0"); // 全放開
    s.belt("~v0 16"); // 只按恢復 → 啟動
    assert_eq!(s.out.belt, vec!["KM998 1", "KM998 3"]);
}

#[test]
fn 空車卡件_對不到包裹_只在堵塞開始記一筆() {
    let mut s = Sim::new();
    // 沒有任何包裹綁在 9 號小車上
    s.sorter("~k9 25 25");
    s.at(500).sorter("~k9 25 25");
    assert_eq!(s.out.jam_records, vec![(9, 25, false)], "堵塞持續的 ~k 不重複記");
    // 安靜超過 2 秒堵塞解除，再卡就是新的一次
    s.at(2500).sorter("~k9 25 25");
    assert_eq!(s.out.jam_records.len(), 2);
}

/// 走完一件到落格完成（R3），回傳這件的 key
fn land_one(s: &mut Sim, slot: u32, cart: u32, barcode: &str) -> u64 {
    s.belt(&format!("~P{slot} 1"));
    let key = s.m.parcels.keys().copied().max().expect("~P 後一定有這件");
    s.at(200).barcode(barcode);
    s.at(300).chute(key, "R3");
    s.at(100).belt(&format!("~L{slot} 25 111"));
    s.at(900).belt(&format!("~O{slot} 1"));
    s.at(14).sorter(&format!("~c{cart} {cart} 410 1"));
    s.at(12).sorter(&format!("~j{cart} 1"));
    s.at(400).sorter(&format!("~g{cart} -1 1"));
    s.at(300).belt(&format!("~E{slot} 1"));
    s.at(700).sorter(&format!("~e{cart}"));
    assert_eq!(s.parcel(key).status, Status::Done);
    s.at(400);
    key
}

#[test]
fn 落格件數累計_到上限提醒_之後每五件再提醒() {
    use super::machine::BagOp;
    let mut s = Sim::new();
    let mut ch = chutes();
    ch.get_mut("R3").unwrap().bag_limit = 2;
    s.m.set_chutes(ch);
    land_one(&mut s, 1, 7, "A1");
    assert!(matches!(s.out.bag_ops.as_slice(), [BagOp::Open { code, seq: 1, .. }] if code == "R3"), "第一件落下才開第一袋: {:?}", s.out.bag_ops);
    assert!(s.out.bag_full.is_empty());
    land_one(&mut s, 2, 8, "A2");
    assert_eq!(s.out.bag_full, vec![("R3".to_string(), 1, 2, 2)], "第 2 件到上限就提醒");
    for (i, (slot, cart)) in [(3u32, 9u32), (4, 10), (5, 11), (6, 12)].iter().enumerate() {
        land_one(&mut s, *slot, *cart, &format!("B{i}"));
    }
    assert_eq!(s.out.bag_full.len(), 1, "3～6 件（超過上限 1～4 件）不再提醒");
    land_one(&mut s, 7, 13, "C7");
    assert_eq!(s.out.bag_full.last().unwrap(), &("R3".to_string(), 1, 7, 2), "超過上限 5 件再提醒一次");
    let view = s.m.bags_view();
    let r3 = view.iter().find(|b| b.code == "R3").unwrap();
    assert_eq!((r3.seq, r3.count, r3.limit), (1, 7, 2));
    let l1 = view.iter().find(|b| b.code == "L1").unwrap();
    assert_eq!((l1.seq, l1.count), (1, 0), "沒落過件的格口顯示第 1 袋 0 件");
}

#[test]
fn 換袋_關上一袋定版件數_開下一袋歸零() {
    use super::machine::BagOp;
    let mut s = Sim::new();
    land_one(&mut s, 1, 7, "A1");
    land_one(&mut s, 2, 8, "A2");
    s.out.bag_ops.clear();
    let ts = s.now;
    s.m.handle(Input::NewBag { code: "R3".into(), by: "web".into() }, &mut s.out);
    assert_eq!(
        s.out.bag_ops,
        vec![
            BagOp::Close { code: "R3".into(), seq: 1, ended_ms: ts, count: 2, closed_by: "web".into() },
            BagOp::Open { code: "R3".into(), seq: 2, started_ms: ts },
        ]
    );
    let r3 = s.m.bags_view().into_iter().find(|b| b.code == "R3").unwrap();
    assert_eq!((r3.seq, r3.count), (2, 0));
    assert!(s.out.logs.iter().any(|l| l.contains("換袋") && l.contains("第 2 袋") && l.contains("上一袋 2 件")), "logs={:?}", s.out.logs);
    land_one(&mut s, 3, 9, "A3");
    assert_eq!(s.m.bags_view().into_iter().find(|b| b.code == "R3").unwrap().count, 1, "新袋從 0 開始數");
    // 沒落過件的格口直接換袋：只開第 1 袋
    s.out.bag_ops.clear();
    s.m.handle(Input::NewBag { code: "L1".into(), by: "web".into() }, &mut s.out);
    assert_eq!(s.out.bag_ops, vec![BagOp::Open { code: "L1".into(), seq: 1, started_ms: s.now }]);
    // 不存在的格口：不動、只記警告
    s.out.bag_ops.clear();
    s.m.handle(Input::NewBag { code: "ZZ".into(), by: "web".into() }, &mut s.out);
    assert!(s.out.bag_ops.is_empty());
}

#[test]
fn 啟動時灌入既有袋子_接著數() {
    let mut s = Sim::new();
    let mut bags = HashMap::new();
    bags.insert("R3".to_string(), super::machine::BagState { seq: 3, started_ms: 5, count: 40 });
    s.m.set_bags(bags);
    land_one(&mut s, 1, 7, "A1");
    let r3 = s.m.bags_view().into_iter().find(|b| b.code == "R3").unwrap();
    assert_eq!((r3.seq, r3.count), (3, 41));
    assert!(s.out.bag_ops.is_empty(), "已有袋子不再開新袋");
}
