//! 給網頁後台看的即時狀態快照（連線、計數）。只有狀態機寫，其他人讀。

use std::sync::{Arc, RwLock};

use serde::Serialize;

use crate::device::Device;

#[derive(Clone, Debug, Default, Serialize)]
pub struct DeviceStatus {
    pub connected: bool,
    pub detail: String,
    pub since_ms: i64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RuntimeSnapshot {
    pub belt: DeviceStatus,
    pub sorter: DeviceStatus,
    pub camera: DeviceStatus,
}

#[derive(Clone, Default)]
pub struct Runtime(Arc<RwLock<RuntimeSnapshot>>);

impl Runtime {
    pub fn snapshot(&self) -> RuntimeSnapshot {
        self.0.read().unwrap().clone()
    }

    pub fn set_device(&self, device: Device, connected: bool, detail: String, ts_ms: i64) {
        let mut g = self.0.write().unwrap();
        let slot = match device {
            Device::Belt => &mut g.belt,
            Device::Sorter => &mut g.sorter,
            Device::Camera => &mut g.camera,
        };
        *slot = DeviceStatus { connected, detail, since_ms: ts_ms };
    }
}
