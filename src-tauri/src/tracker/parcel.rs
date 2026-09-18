//! 單一包裹的狀態與時間戳。

use serde::Serialize;

use crate::protocol::Cid;

/// 分揀狀態；數值沿用舊系統，歷史資料與現場人員的認知不必改。序列化為數字，與資料表一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Status {
    Init = 1,
    Received = 2,
    Done = 3,
    Lost = 4,
    Blocked = 5,
    BlockedThenTaken = 6,
    Cancelled = 7,
    TriggerNg = 8,
}

impl Serialize for Status {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i32(self.code())
    }
}

impl Status {
    pub fn code(self) -> i32 {
        self as i32
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Init => "初始化",
            Status::Received => "收件",
            Status::Done => "完成",
            Status::Lost => "失去追蹤",
            Status::Blocked => "堵塞",
            Status::BlockedThenTaken => "堵塞後取走",
            Status::Cancelled => "指令取消",
            Status::TriggerNg => "觸發異常",
        }
    }

    /// 終態：之後不再有訊號會改變它
    pub fn is_final(self) -> bool {
        matches!(self, Status::Done | Status::Lost | Status::BlockedThenTaken | Status::Cancelled | Status::TriggerNg)
    }
}

/// 格口決定的來源
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChuteSource {
    /// 尚未決定
    Pending,
    /// 中介機回覆
    Api,
    /// 中介機沒在時限內回覆，走預設口
    Timeout,
    /// 相機讀不到條碼
    NoRead,
    /// 中介機回錯或回無效格口，走預設口
    Default,
    /// 網頁手動指定（快速分揀）
    Manual,
}

/// 已下載好的面單，跟著格口決定一起送進狀態機；狀態機接受決定後才交給列印，
/// 決定被拒（回覆太晚、包裹已走預設口）就一起丟掉，不會印出沒有包裹可貼的面單
#[derive(Clone)]
pub struct LabelPayload {
    pub bytes: Vec<u8>,
    pub print_profile: Option<String>,
    pub is_error_label: bool,
}

impl std::fmt::Debug for LabelPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LabelPayload").field("bytes", &self.bytes.len()).field("print_profile", &self.print_profile).field("is_error_label", &self.is_error_label).finish()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ChuteDecision {
    pub code: String,
    pub cid: Cid,
    pub source: ChuteSource,
    pub response_id: Option<i64>,
    pub decided_ms: i64,
    /// 走預設口／帶錯誤面單的原因代碼（見 migration 0003）；正常給格口為 None
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Parcel {
    /// 記憶體內的鍵（單調遞增），DB 的 id 由 store 對應
    pub key: u64,
    pub ulid: String,
    pub barcode: Option<String>,
    pub slot: u32,
    pub cart: Option<u32>,
    pub status: Status,
    pub chute: Option<ChuteDecision>,
    pub ir_length: Option<i32>,
    pub gap: Option<i32>,
    pub block_pos: Option<i32>,
    pub lost_pos: Option<i32>,
    pub p_ms: i64,
    pub l_ms: Option<i64>,
    pub o_ms: Option<i64>,
    pub belt_e_ms: Option<i64>,
    pub bind_ms: Option<i64>,
    pub kn_ms: Option<i64>,
    pub c_ms: Option<i64>,
    pub j_ms: Option<i64>,
    pub g_ms: Option<i64>,
    pub e_ms: Option<i64>,
    pub k_ms: Option<i64>,
    pub u_ms: Option<i64>,
    pub kx_ms: Option<i64>,
    pub ended_ms: Option<i64>,
    /// 曾經堵塞（`~u` 時用來分辨 4 與 6）
    pub was_blocked: bool,
    /// 皮帶已 `~E` 但頭部一直沒收到 `~j/~g`（Kx 判斷用）
    pub belt_stop_sent: bool,
}

impl Parcel {
    pub fn new(key: u64, slot: u32, p_ms: i64) -> Self {
        Self {
            key,
            ulid: ulid::Ulid::generate().to_string(),
            barcode: None,
            slot,
            cart: None,
            status: Status::Init,
            chute: None,
            ir_length: None,
            gap: None,
            block_pos: None,
            lost_pos: None,
            p_ms,
            l_ms: None,
            o_ms: None,
            belt_e_ms: None,
            bind_ms: None,
            kn_ms: None,
            c_ms: None,
            j_ms: None,
            g_ms: None,
            e_ms: None,
            k_ms: None,
            u_ms: None,
            kx_ms: None,
            ended_ms: None,
            was_blocked: false,
            belt_stop_sent: false,
        }
    }

    pub fn barcode_or_noread(&self) -> &str {
        self.barcode.as_deref().unwrap_or(crate::device::camera::NO_READ)
    }

    pub fn is_ended(&self) -> bool {
        self.ended_ms.is_some()
    }
}
