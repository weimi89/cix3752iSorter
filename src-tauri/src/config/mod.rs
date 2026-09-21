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
    pub camera_ftp: CameraFtpConfig,
    pub middleware: MiddlewareConfig,
    pub ng: NgConfig,
    pub print: PrintConfig,
    pub emergency_buttons: Vec<EmergencyButton>,
    pub web_access: WebAccessConfig,
}

/// 網頁後台的對外存取設定。
///
/// 內網來源（現場電腦、工控機、手機）一律免登入；**外網來源要輸入共用密碼**，
/// 通過後與坐在現場有同等權限。密碼雜湊不放這裡 —— 這份設定會被 `GET /api/config`
/// 整包回給前端，雜湊跟著跑到瀏覽器沒有必要。密碼另存資料庫 `app_setting`。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WebAccessConfig {
    /// 對外存取總開關。關閉時非內網來源一律擋掉，連登入頁都不給 ——
    /// 預設關閉，要對外開放是明確的決定，不該因為裝了新版就自動生效。
    pub enabled: bool,
    /// 視為內網的網段（CIDR）。命中者免登入。
    ///
    /// 判斷一律以 TCP 連線的來源位址為準，**不看 X-Forwarded-For** ——
    /// 這台機器直接對外，標頭是任何人都能偽造的，信了等於整道門形同虛設。
    pub lan_cidrs: Vec<String>,
    /// 登入後多久要重新輸入密碼（小時）
    pub session_hours: u32,
    /// 同一來源連續失敗幾次就鎖住
    pub max_fail_attempts: u32,
    /// 鎖多久（分鐘）
    pub lock_minutes: u32,
}

impl Default for WebAccessConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            lan_cidrs: vec![
                "127.0.0.0/8".into(),
                "10.0.0.0/8".into(),
                "172.16.0.0/12".into(),
                "192.168.0.0/16".into(),
                "169.254.0.0/16".into(),
                "::1/128".into(),
                "fc00::/7".into(),
                "fe80::/10".into(),
            ],
            session_hours: 8,
            max_fail_attempts: 5,
            lock_minutes: 15,
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

/// 讀碼站照片：讀碼器每件拍的圖用 FTP 上傳到本程式，當「這件我們有收到」的證據。
///
/// 本程式自己當 FTP 伺服器（不另裝 vsftpd、不用 root），收到原圖立刻縮成證據圖存檔，
/// 原圖不留——20MP 一張 2–3 MB，一天五千多件會把硬碟吃光。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CameraFtpConfig {
    /// 關掉就不開 FTP 埠（讀碼器那邊也要把 FTP 關掉，不然它會一直連不上）
    pub enabled: bool,
    /// FTP 監聽位址；預設 2121 避開需要 root 的 21
    pub listen: String,
    pub username: String,
    pub password: String,
    /// 被動模式（PASV）資料連線用的埠範圍；工控機開著防火牆，固定一段才放得了行。兩個都 0 = 隨機埠
    pub passive_port_min: u16,
    pub passive_port_max: u16,
    /// 照片對回包裹的時間窗口：照片上傳完成時間往前找這麼多毫秒內綁到條碼的件
    pub match_window_ms: i64,
    /// 證據圖長邊像素；0 = 不縮、存原圖（硬碟會很快滿）
    pub max_edge_px: u32,
    /// 證據圖 JPEG 品質 1–100
    pub jpeg_quality: u8,
    /// 讀碼失敗的件另外保留原圖（看清楚為什麼讀不到）
    pub keep_original_noread: bool,
    /// 照片保留天數；0 = 不清理。與包裹資料的保留天數分開——證據要留得比訊號久
    pub retention_days: u32,
    /// 照片存放目錄（絕對路徑）；空白 = 資料目錄下的 `images`。要把圖檔跟資料庫分開放（另一顆硬碟）就填這裡
    pub images_dir: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MiddlewareConfig {
    /// cix3752iLabelPrint 的本地 HTTP server
    pub base_url: String,
    /// `GET /api/parcel` 逾時；超過就走預設口。
    /// 真正的截止點是包裹到交接點（`~O`，中位 1.3 秒、皮帶停過會更久）：到那時還沒回覆才用預設口，
    /// 之後才到的回覆只記「回覆太晚」。所以這個值只是保底，不必卡在 1.2 秒把還來得及的回覆砍掉
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
            camera_ftp: CameraFtpConfig::default(),
            middleware: MiddlewareConfig::default(),
            ng: NgConfig::default(),
            print: PrintConfig::default(),
            emergency_buttons: Vec::new(),
            web_access: WebAccessConfig::default(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            // 舊廠商程式的網頁後台占著 8080，並行期間不能撞埠
            bind: "0.0.0.0:18090".into(),
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
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

impl Default for CameraFtpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            listen: "0.0.0.0:2121".into(),
            username: "sorter".into(),
            password: "sorter".into(),
            passive_port_min: 50000,
            passive_port_max: 50100,
            match_window_ms: 5000,
            max_edge_px: 1600,
            jpeg_quality: 80,
            keep_original_noread: true,
            retention_days: 90,
            images_dir: String::new(),
        }
    }
}

impl Default for MiddlewareConfig {
    fn default() -> Self {
        Self {
            base_url: "http://192.168.0.37:18080/".into(),
            parcel_timeout_ms: 2500,
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
    /// 更新一律排隊：兩個人同時存檔時，後到的那份是以「拿到鎖之後」的現況為基礎算出來的，
    /// 不會用過期的快照把別人剛存的蓋回去（外網能不能改門鎖的判斷就靠這一點）
    write_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
}

impl ConfigHandle {
    pub fn new(path: PathBuf, cfg: AppConfig) -> Self {
        let (tx, _) = watch::channel(cfg);
        Self { path, tx, write_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())) }
    }

    pub fn current(&self) -> AppConfig {
        self.tx.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<AppConfig> {
        self.tx.subscribe()
    }

    /// 落檔成功才廣播；落檔失敗時記憶體內的值也不變，避免「畫面顯示已存、重啟後不見」。
    pub async fn update(&self, cfg: AppConfig) -> anyhow::Result<()> {
        self.update_with(|_| cfg).await
    }

    /// 以「拿到寫入鎖當下的現況」為基礎產生新設定再落檔；要保留現況某些欄位（例如外網來源
    /// 不得動的 `web_access`）必須走這條，先讀 `current()` 再 `update()` 中間會被別人插隊。
    pub async fn update_with(&self, f: impl FnOnce(&AppConfig) -> AppConfig) -> anyhow::Result<()> {
        let _guard = self.write_lock.lock().await;
        let cfg = f(&self.tx.borrow());
        cfg.save(&self.path).await?;
        self.tx.send_replace(cfg);
        Ok(())
    }

    /// 不改內容、只讓訂閱者重跑一次（例如格口表換了要狀態機重載）；不落檔、不讀舊值再寫回
    pub fn touch(&self) {
        self.tx.send_modify(|_| {});
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
