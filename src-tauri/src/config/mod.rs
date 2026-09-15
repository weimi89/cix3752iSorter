//! 系統設定（`config.toml`）。
//!
//! 對應舊系統的 `data/conf.json`（裝置、NG 規則、燈號）與 `gkconfig.json`（列印 profile）。
//! 格口對照（代號 ↔ CID ↔ 印表機）不在這裡，放資料庫 `chutes` 表。
//!
//! 設定以 `watch` channel 廣播：改設定後各裝置 task 會收到新值，連線類參數
//! （位址）變更時自行重連；其餘參數即時生效，不必重啟。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::fs_atomic;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub general: GeneralConfig,
    pub belt: BeltConfig,
    pub sorter: SorterConfig,
    pub sysled: SysLedConfig,
    pub camera: CameraConfig,
    pub middleware: MiddlewareConfig,
    pub ng: NgConfig,
    pub print: PrintConfig,
    pub update: UpdateConfig,
    pub emergency_buttons: Vec<EmergencyButton>,
}

/// 自動更新（GitHub Release 的 latest.json）
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UpdateConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub check_interval_min: u64,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: "https://github.com/weimi89/cix3752iSorter/releases/latest/download/latest.json".into(),
            check_interval_min: 60,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ServerConfig {
    /// 網頁後台監聽位址
    pub bind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GeneralConfig {
    pub title: String,
    pub machine: String,
    /// 包裹與事件保留天數；0 = 不清理
    pub retention_days: u32,
    /// 拿不到格口時的預設格口代號（查 `chutes` 表）
    pub default_chute: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BeltConfig {
    pub addr: String,
    /// true = 常動（用 run 指令啟動）；false = 節能（用 auto）
    pub default_run: bool,
    pub cmd: BeltCommands,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BeltCommands {
    pub auto: String,
    pub run: String,
    pub stop: String,
    pub reset: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SorterConfig {
    pub addr: String,
    /// 分揀機速度，帶進 `Kn` 指令
    pub speed: u32,
    /// 小車／設備數量
    pub number: u32,
    /// 光電數量（IR 檢查頁用）
    pub ir_num: u32,
    /// `~I` 訊號是否建立包裹（皮帶沒有 `~P` 的機型才開）
    pub init_parcel_on_i: bool,
    /// `~u` 丟失是否當作完成處理
    pub u_as_done: bool,
    /// `Kn` 後多久收不到 `~c` 就停線（毫秒）
    pub c_timeout_ms: u64,
    /// 收到 `~c` 後多久沒有 `~j/~g` 且皮帶已 `~E` 就發 `Kx`（毫秒）
    pub kx_after_e_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SysLedConfig {
    /// 燈號走哪條線："sorter" 或 "belt"（與該裝置共用連線）
    pub via: String,
    pub cmd: SysLedCommands,
    pub alarm_on_start: bool,
    pub alarm_on_lost: bool,
    pub alarm_on_block: bool,
    pub alarm_on_cancel: bool,
    /// 異常燈維持幾毫秒後回綠
    pub alarm_hold_ms: u64,
    /// 綠燈多久無新件轉閒置燈
    pub idle_after_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SysLedCommands {
    pub off: String,
    pub red: String,
    pub green: String,
    pub idle: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CameraConfig {
    /// 相機讀碼站連進來的 TCP 監聽位址
    pub listen: String,
    /// 條碼時間相對 `~P` 的綁定窗口（毫秒）
    pub bind_floor_ms: i64,
    pub bind_ceiling_ms: i64,
    /// 條碼通常在 `~P` 後多久到（現場實測中位 226ms）；窗口內有多件候選時挑最接近這個值的
    pub bind_expected_ms: i64,
    /// 同一條碼多久內不重複觸發
    pub dedup_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MiddlewareConfig {
    /// cix3752iLabelPrint 的本地 HTTP server
    pub base_url: String,
    /// `GET /api/parcel` 逾時；超過就走預設口
    pub parcel_timeout_ms: u64,
    pub label_timeout_ms: u64,
    pub report_timeout_ms: u64,
    pub alert_timeout_ms: u64,
    /// 同一格口卡件警報最短間隔
    pub jam_alert_throttle_ms: u64,
    /// 同一格口多久沒再收到 `~k` 視為那次堵塞結束，下一個 `~k` 立刻再告警
    pub jam_alert_reset_ms: u64,
}

/// 異常時是否停線（對應舊 `conf.ng`）
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NgConfig {
    pub too_close: bool,
    pub on_i: bool,
    pub on_block: bool,
    pub on_lost: bool,
    pub on_cancel: bool,
    /// `~L` 間距小於此值視為過近
    pub min_gap: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PrintConfig {
    pub dpi: u32,
    /// 送印延遲 = (格口號 - 1) * 此值
    pub per_chute_delay_ms: u64,
    pub profiles: std::collections::BTreeMap<String, PrintProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PrintProfile {
    /// TSPL 前置指令（`DIRECTION 1\r\nSPEED 6\r\nDENSITY 11`）
    pub cmd: String,
    /// BITMAP 起點
    pub org: String,
    pub retry: u32,
    pub retry_interval_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EmergencyButton {
    pub describe: String,
    /// "belt" / "sorter"：哪條線的 `~v` 訊號
    pub device: String,
    /// `~v<m2>` 的 m2
    pub m2: u32,
    /// 8 位元中的第幾位（0 = 最高位，對應舊設定）
    pub bit: u8,
    /// 該位由 0 變 1 時的動作："stop" 停線、"start" 啟動、
    /// "estop" 急停（停線並鎖住，放開前「急停恢復」無效）、"estop_release" 急停恢復（急停沒鎖住才啟動）
    pub action: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            general: GeneralConfig::default(),
            belt: BeltConfig::default(),
            sorter: SorterConfig::default(),
            sysled: SysLedConfig::default(),
            camera: CameraConfig::default(),
            middleware: MiddlewareConfig::default(),
            ng: NgConfig::default(),
            print: PrintConfig::default(),
            update: UpdateConfig::default(),
            emergency_buttons: Vec::new(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:8080".into(),
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            title: "物流貓智能分揀系統".into(),
            machine: "001".into(),
            retention_days: 15,
            default_chute: "RS".into(),
        }
    }
}

impl Default for BeltConfig {
    fn default() -> Self {
        Self {
            addr: "192.168.177.100:10006".into(),
            default_run: true,
            cmd: BeltCommands::default(),
        }
    }
}

impl Default for BeltCommands {
    fn default() -> Self {
        Self {
            auto: "KM998 3".into(),
            run: "KM998 3".into(),
            stop: "KM998 1".into(),
            reset: "KM999".into(),
        }
    }
}

impl Default for SorterConfig {
    fn default() -> Self {
        Self {
            addr: "192.168.177.198:10006".into(),
            speed: 100,
            number: 8,
            ir_num: 38,
            init_parcel_on_i: false,
            u_as_done: false,
            c_timeout_ms: 300,
            kx_after_e_ms: 100,
        }
    }
}

impl Default for SysLedConfig {
    fn default() -> Self {
        Self {
            via: "sorter".into(),
            cmd: SysLedCommands::default(),
            alarm_on_start: true,
            alarm_on_lost: true,
            alarm_on_block: true,
            alarm_on_cancel: true,
            alarm_hold_ms: 3000,
            idle_after_ms: 10_000,
        }
    }
}

impl Default for SysLedCommands {
    fn default() -> Self {
        Self {
            off: "KL 0 2 3000".into(),
            red: "KL 0 2 3200".into(),
            green: "KL 0 2 3010".into(),
            idle: "KL 0 2 3001".into(),
        }
    }
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8051".into(),
            bind_floor_ms: -100,
            bind_ceiling_ms: 2000,
            bind_expected_ms: 226,
            dedup_ms: 5000,
        }
    }
}

impl Default for MiddlewareConfig {
    fn default() -> Self {
        Self {
            base_url: "http://192.168.0.37:18080/".into(),
            parcel_timeout_ms: 1200,
            label_timeout_ms: 1200,
            report_timeout_ms: 5000,
            alert_timeout_ms: 3000,
            jam_alert_throttle_ms: 20_000,
            jam_alert_reset_ms: 5_000,
        }
    }
}

impl Default for NgConfig {
    fn default() -> Self {
        Self {
            too_close: false,
            on_i: false,
            on_block: true,
            on_lost: false,
            on_cancel: false,
            min_gap: 50,
        }
    }
}

impl Default for PrintConfig {
    fn default() -> Self {
        let mut profiles = std::collections::BTreeMap::new();
        profiles.insert(
            "PAPER-01#95*195".into(),
            PrintProfile { cmd: "DIRECTION 1".into(), ..Default::default() },
        );
        profiles.insert(
            "PAPER-01#100*100".into(),
            PrintProfile { cmd: "DIRECTION 1".into(), ..Default::default() },
        );
        profiles.insert(
            "PAPER-01#100*150".into(),
            PrintProfile { cmd: "DIRECTION 1\r\nSPEED 6\r\nDENSITY 11".into(), ..Default::default() },
        );
        Self { dpi: 203, per_chute_delay_ms: 400, profiles }
    }
}

impl Default for PrintProfile {
    fn default() -> Self {
        Self {
            cmd: "DIRECTION 1\r\nREFERENCE 0,0".into(),
            org: "0,0".into(),
            retry: 30,
            retry_interval_ms: 200,
        }
    }
}

impl AppConfig {
    /// 讀取設定檔；檔案不存在時以預設值建立一份，讓現場第一次啟動就有可編輯的範本。
    pub async fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        match tokio::fs::read_to_string(path).await {
            Ok(text) => {
                let cfg: AppConfig = toml::from_str(&text)
                    .map_err(|e| anyhow::anyhow!("設定檔 {} 解析失敗: {e}", path.display()))?;
                Ok(cfg)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let cfg = AppConfig::default();
                cfg.save(path).await?;
                tracing::warn!(path = %path.display(), "設定檔不存在，已用預設值建立");
                Ok(cfg)
            }
            Err(e) => Err(anyhow::anyhow!("設定檔 {} 讀取失敗: {e}", path.display())),
        }
    }

    pub async fn save(&self, path: &Path) -> anyhow::Result<()> {
        let text = toml::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        fs_atomic::write_async(path, text.as_bytes()).await?;
        Ok(())
    }
}

/// 設定的共享句柄：讀取端 `borrow()` 拿快照，改設定走 `update()` 同時落檔並廣播。
#[derive(Clone)]
pub struct ConfigHandle {
    path: PathBuf,
    tx: watch::Sender<AppConfig>,
}

impl ConfigHandle {
    pub fn new(path: PathBuf, cfg: AppConfig) -> Self {
        let (tx, _) = watch::channel(cfg);
        Self { path, tx }
    }

    pub fn current(&self) -> AppConfig {
        self.tx.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<AppConfig> {
        self.tx.subscribe()
    }

    /// 落檔成功才廣播；落檔失敗時記憶體內的值也不變，避免「畫面顯示已存、重啟後不見」。
    pub async fn update(&self, cfg: AppConfig) -> anyhow::Result<()> {
        cfg.save(&self.path).await?;
        self.tx.send_replace(cfg);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 預設值可往返_toml() {
        let cfg = AppConfig::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: AppConfig = toml::from_str(&text).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn 缺少區段時用預設補齊() {
        // 現場手改設定檔只寫了皮帶位址，其餘不能因此變成空值
        let cfg: AppConfig = toml::from_str("[belt]\naddr = \"10.0.0.1:1\"\n").unwrap();
        assert_eq!(cfg.belt.addr, "10.0.0.1:1");
        assert_eq!(cfg.belt.cmd.stop, "KM998 1");
        assert_eq!(cfg.sorter.speed, 100);
    }
}
