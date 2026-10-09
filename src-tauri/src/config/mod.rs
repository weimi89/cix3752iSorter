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

/// 同碼再進線攔截秒數的上限；查格口用的記憶體也只保留這麼久的包裹，兩邊必須同一個數字
pub const REENTRY_HOLD_MAX_MS: i64 = 60_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GeneralConfig {
    /// 包裹與事件保留天數；0 = 不清理
    pub retention_days: u32,
    /// 拿不到格口時的預設格口代號（查 `chutes` 表）
    pub default_chute: String,
    /// 同一條碼在這段時間內剛正常分過又進線 → 不問中介機、直接走異常口讓人看（讀到鄰件條碼的件）。
    /// 9/21 實測：讀到鄰件的間隔 1.7–5.8 秒，正常撿回重投最快 7.9 秒——預設 6 秒落在中間；設到 8 秒以上會攔到正常重投。0 = 關閉
    pub reentry_hold_ms: i64,
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
    /// 相機畫面像素尺寸；同幀有多個合法 QR 時取離畫面中心最近的（貼邊的是鄰件）。任一邊 0 = 不用座標，照字典序。
    /// 預設 0：解析度填錯（例如實際 2592×1944 卻用 5440×3648）會把「中心」算到畫面外，固定挑靠角落的那個、比不看座標更糟，
    /// 所以一定要人依相機原圖尺寸填（桃園 MV-ID6200M 原圖 5440×3648，量自 FTP 上傳的照片）
    pub frame_width: u32,
    pub frame_height: u32,
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
    /// 證據圖長邊像素；0 = 不縮、存原圖。業主 9/22 裁示全部存原圖（縮圖看不清楚），
    /// 原圖一張約 620 KB、一天 9 千件≈5.7 GB，保留天數要跟著硬碟算（203 GB 約放 30 天）
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
    /// 分揀機自己印面單（貼標機接在工控機 USB）。關閉時面單一律由中介機處理：
    /// 查格口不抓面單、不送印，就算格口對照還留著印表機埠位也不印
    pub enabled: bool,
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
            reentry_hold_ms: 6000,
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
            frame_width: 0,
            frame_height: 0,
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
            max_edge_px: 0,
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
        Self { enabled: false, dpi: 203, per_chute_delay_ms: 400, profiles }
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

/// 一個超出範圍的欄位：名稱與值用來比對「原本就是這個錯誤值」
struct RangeViolation {
    label: &'static str,
    value: i64,
    message: String,
}

impl AppConfig {
    /// 設定頁有上下限的欄位，存檔時再檢查一次——設定檔可以手改、API 可以直接打，只靠畫面擋不住。
    /// 範圍與名稱和設定頁一致；超出範圍的值會讓每件都走預設口、綁不到條碼或一直停線
    pub fn check_ranges(&self) -> Result<(), String> {
        match self.range_violations().into_iter().next() {
            Some(v) => Err(v.message),
            None => Ok(()),
        }
    }

    /// 存檔用：只擋這次新改出來的超範圍值。設定檔手改出來、原本就在生效的錯誤值維持原狀
    /// （開機時已記錄錯誤）——那個欄位畫面上不一定看得到，擋下來現場就什麼都存不了
    pub fn check_new_violations(&self, old: &AppConfig) -> Result<(), String> {
        let before = old.range_violations();
        match self.range_violations().into_iter().find(|v| !before.iter().any(|b| b.label == v.label && b.value == v.value)) {
            Some(v) => Err(v.message),
            None => Ok(()),
        }
    }

    fn range_violations(&self) -> Vec<RangeViolation> {
        let mut out = Vec::new();
        let mut check = |label: &'static str, v: i64, lo: i64, hi: i64, unit: &str| {
            if !(lo..=hi).contains(&v) {
                out.push(RangeViolation { label, value: v, message: format!("{label}必須在 {lo}–{hi}{unit}") });
            }
        };
        let (m, s, c) = (&self.middleware, &self.sorter, &self.camera);
        check("查格口逾時", m.parcel_timeout_ms as i64, 200, 5_000, " 毫秒");
        check("下載面單逾時", m.label_timeout_ms as i64, 200, 10_000, " 毫秒");
        check("回報逾時", m.report_timeout_ms as i64, 500, 30_000, " 毫秒");
        check("同格口卡件告警間隔", m.jam_alert_throttle_ms as i64, 1_000, 600_000, " 毫秒");
        check("速度", s.speed as i64, 1, 300, "");
        check("小車數", s.number as i64, 1, 26, "");
        check("光電數", s.ir_num as i64, 0, 200, "");
        check("下指令後等待受理", s.c_timeout_ms as i64, 50, 5_000, " 毫秒");
        check("離開皮帶後等待上車", s.kx_after_e_ms as i64, 0, 5_000, " 毫秒");
        check("綁碼窗口下限", c.bind_floor_ms, -5_000, 0, " 毫秒");
        check("綁碼窗口上限", c.bind_ceiling_ms, 100, 10_000, " 毫秒");
        check("同碼去重", c.dedup_ms as i64, 0, 60_000, " 毫秒");
        check("資料保留天數", self.general.retention_days as i64, 0, 365, "");
        check("DPI", self.print.dpi as i64, 72, 600, "");
        check("每格口送印延遲", self.print.per_chute_delay_ms as i64, 0, 10_000, " 毫秒");
        out
    }

    /// 讀取設定檔；檔案不存在時以預設值建立一份，讓現場第一次啟動就有可編輯的範本。
    pub async fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        match tokio::fs::read_to_string(path).await {
            Ok(text) => {
                let cfg: AppConfig = toml::from_str(&text)
                    .map_err(|e| anyhow::anyhow!("設定檔 {} 解析失敗: {e}", path.display()))?;
                // 手改出範圍外的值照常啟動（擋下來現場整條線就停了），但要留下紀錄；到設定頁存檔時會被要求改正
                if let Err(e) = cfg.check_ranges() {
                    tracing::error!(path = %path.display(), "設定檔有超出範圍的值：{e}");
                }
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
        fs_atomic::write_async(path, text.as_bytes(), fs_atomic::Durability::Synced).await?;
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
        self.try_update_with(|cur| Ok::<_, std::convert::Infallible>(f(cur))).await.map(|_| ())
    }

    /// 同 [`update_with`]，`f` 回 `Err` 就不寫入。要對照現況的檢查一定要放在 `f` 裡（寫入鎖之內）：
    /// 鎖外先檢查的話，兩個人同時存，拿舊快照的那一份會通過檢查，再把別人剛改好的值蓋回去
    pub async fn try_update_with<E>(&self, f: impl FnOnce(&AppConfig) -> Result<AppConfig, E>) -> anyhow::Result<Result<(), E>> {
        let _guard = self.write_lock.lock().await;
        let cfg = match f(&self.tx.borrow()) {
            Ok(c) => c,
            Err(e) => return Ok(Err(e)),
        };
        cfg.save(&self.path).await?;
        self.tx.send_replace(cfg);
        Ok(Ok(()))
    }

    /// 設定頁送來的整份設定：在寫入鎖內對照當下的設定檢查範圍（只擋新引入的錯誤值）再寫入。
    /// `keep_web_access`：外網來源不管送來的網頁存取設定長怎樣，一律保留寫入當下的現況
    /// ——只比對不覆蓋的話，外網拿著舊快照就能把內網剛改好的設定蓋回去
    pub async fn save_request(&self, next: AppConfig, keep_web_access: bool) -> anyhow::Result<Result<(), String>> {
        self.try_update_with(move |current| {
            let mut next = next;
            if keep_web_access {
                next.web_access = current.web_access.clone();
            }
            next.check_new_violations(current)?;
            Ok(next)
        })
        .await
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

    #[test]
    fn 設定檔原本就有超範圍的值_存別的欄位不可被擋_改成另一個超範圍值要擋() {
        // 畫面上不一定看得到那個欄位（例如印單關閉時不顯示下載面單逾時），擋下來現場就什麼都存不了
        let mut old = AppConfig::default();
        old.middleware.label_timeout_ms = 50;
        let mut next = old.clone();
        next.sorter.speed = 120;
        assert_eq!(next.check_new_violations(&old), Ok(()), "原本的錯誤值沒動，存別的欄位照常");
        let mut next = old.clone();
        next.middleware.label_timeout_ms = 60;
        assert_eq!(next.check_new_violations(&old), Err("下載面單逾時必須在 200–10000 毫秒".into()), "改成另一個錯誤值要擋");
        let mut next = old.clone();
        next.middleware.parcel_timeout_ms = 0;
        assert_eq!(next.check_new_violations(&old), Err("查格口逾時必須在 200–5000 毫秒".into()), "新改出來的錯誤值要擋");
        assert_eq!(AppConfig::default().check_new_violations(&AppConfig::default()), Ok(()));
    }

    #[test]
    fn 存檔範圍檢查_與設定頁一致_超出就擋並說是哪個欄位() {
        assert_eq!(AppConfig::default().check_ranges(), Ok(()), "預設值一定合法");
        let mut c = AppConfig::default();
        c.middleware.parcel_timeout_ms = 0; // 0 = 每次查格口立刻逾時，每件都走預設口
        assert_eq!(c.check_ranges(), Err("查格口逾時必須在 200–5000 毫秒".into()));
        let mut c = AppConfig::default();
        c.camera.bind_floor_ms = 100; // 下限大於 0：條碼比 ~P 早到的件全部綁不到
        assert!(c.check_ranges().unwrap_err().contains("綁碼窗口下限"));
        let mut c = AppConfig::default();
        c.sorter.c_timeout_ms = 10; // 比分揀機正常受理（中位 14ms）還短：每件都會被當成沒受理而停線
        assert!(c.check_ranges().unwrap_err().contains("下指令後等待受理"));
        let mut c = AppConfig::default();
        c.sorter.speed = 0;
        assert!(c.check_ranges().unwrap_err().contains("速度"));
        let mut c = AppConfig::default();
        c.general.retention_days = 400;
        assert_eq!(c.check_ranges(), Err("資料保留天數必須在 0–365".into()));
        let mut c = AppConfig::default();
        c.middleware.report_timeout_ms = 30_000;
        c.camera.bind_ceiling_ms = 100;
        assert_eq!(c.check_ranges(), Ok(()), "邊界值本身可以");
    }


    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn 兩人同時存設定_舊快照不可把別人剛改正的錯誤值寫回去() {
        let dir = std::env::temp_dir().join(format!("cfgrace-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut bad = AppConfig::default();
        bad.middleware.label_timeout_ms = 50;
        let handle = ConfigHandle::new(dir.join("config.toml"), bad.clone());
        // B 把錯誤值改正：寫入鎖握著一段時間
        let b = {
            let handle = handle.clone();
            tokio::spawn(async move {
                handle
                    .update_with(|cur| {
                        std::thread::sleep(std::time::Duration::from_millis(400));
                        let mut next = cur.clone();
                        next.middleware.label_timeout_ms = 200;
                        next
                    })
                    .await
                    .unwrap();
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        // A 拿的是 B 改正前的快照（50），只改了別的欄位
        let mut a = bad.clone();
        a.sorter.speed = 120;
        let r = handle.save_request(a, false).await.unwrap();
        b.await.unwrap();
        assert!(r.is_err(), "A 寫入當下 50 已經是新引入的錯誤值，要擋");
        assert_eq!(handle.current().middleware.label_timeout_ms, 200);
    }
}
