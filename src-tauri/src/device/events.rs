//! 裝置層送給包裹狀態機的事件。所有裝置匯進同一條 channel，狀態機單執行緒處理。

use crate::protocol::{BeltSignal, SorterSignal};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Device {
    Belt,
    Sorter,
    Camera,
}

#[derive(Clone, Debug)]
pub enum DeviceEvent {
    /// 連線狀態變化（相機端為「有無讀碼站連著」）
    State { device: Device, connected: bool, detail: String, ts_ms: i64 },
    Belt { sig: BeltSignal, raw: String, ts_ms: i64 },
    Sorter { sig: SorterSignal, raw: String, ts_ms: i64 },
    /// 讀碼站送來的一幀已挑選出的條碼；`NoRead` 表示這一幀讀不到
    Barcode { code: String, raw: String, ts_ms: i64 },
}
