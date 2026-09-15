//! 自動更新：對齊 cix3752iLabelPrint 的流程（GitHub Release 的 `latest.json` → 下載 → 校驗 → 換掉執行檔 → 重啟）。
//!
//! 這條是 `--headless`（supervisor／systemd）用的；桌面模式由 Tauri updater 外掛裝 .deb。兩條路讀同一份
//! `latest.json`，靠 `platforms` 的鍵區分（見 [`platform_tag`]）：
//! 1. 讀 `latest.json`（GHA 發版時產生），比版本號
//! 2. 串流下載只含執行檔的 tar.gz 到 `data/update/`，同時推 SSE 進度
//! 3. 比對 SHA-256（`latest.json` 裡帶的），不符就丟掉
//! 4. 解出 `sorter` 寫到 `<執行檔>.new`，`rename` 蓋掉正在跑的檔（Linux 允許）
//! 5. 一秒後結束行程，交給 supervisor／systemd 的自動重啟拉起新版
//!
//! 工控機沒有外網時，網頁也能直接上傳 tar.gz 走同一條 3→5。
//! 執行檔動態連結 webkit 棧（20.04 是自編到 /usr/local 的那套），換檔前的 `--version` 自檢載入不了
//! 就會被擋下，所以棧要升版時必須重跑完整安裝包的 install.sh，不能只靠這裡換檔。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::db::DbPool;
use crate::event_bus;
use crate::event_log::{self, Level};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 這台機器在 `latest.json` `platforms` 裡對應的鍵前綴，例如 `linux-x86_64-ubuntu-20.04`。
///
/// Linux 一定帶 distro：20.04 的 .deb 連的是自編 glib/webkit 棧、22.04+ 連系統套件，
/// 兩邊的 .deb 與執行檔不能互換，發版時是三個 distro 各出一份。桌面模式把這個字串交給
/// Tauri updater 當 target（抓 `.deb`），headless 再接 `-headless`（抓只含執行檔的 tar.gz）。
/// distro 讀不出來（非 Ubuntu／Debian 系或 `/etc/os-release` 缺欄位）回 `None`，兩邊都視為「沒有可用更新」。
pub fn platform_tag() -> Option<String> {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "windows",
        _ => return None,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        "x86" => "i686",
        "arm" => "armv7",
        _ => return None,
    };
    if os != "linux" {
        return Some(format!("{os}-{arch}"));
    }
    let distro = linux_distro(&std::fs::read_to_string("/etc/os-release").ok()?)?;
    Some(format!("{os}-{arch}-{distro}"))
}

/// headless 用的鍵：`<platform_tag>-headless`
pub fn headless_platform_key() -> Option<String> {
    platform_tag().map(|t| format!("{t}-headless"))
}

/// 從 `/etc/os-release` 內容組出 `ubuntu-20.04` 這種字串（`ID` + `VERSION_ID`，兩者缺一回 `None`）
fn linux_distro(os_release: &str) -> Option<String> {
    let mut id = None;
    let mut version = None;
    for line in os_release.lines() {
        let (k, v) = match line.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        let v = v.trim().trim_matches('"');
        match k.trim() {
            "ID" => id = Some(v.to_string()),
            "VERSION_ID" => version = Some(v.to_string()),
            _ => {}
        }
    }
    Some(format!("{}-{}", id?, version?))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LatestJson {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub pub_date: String,
    pub platforms: std::collections::BTreeMap<String, PlatformAsset>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlatformAsset {
    pub url: String,
    /// headless 換檔靠這個校驗；桌面用的 .deb 項目另有 Tauri updater 的 `signature`（這裡不理）
    #[serde(default)]
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub available: bool,
    pub notes: String,
    pub pub_date: String,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub checked_at: String,
}

#[derive(Clone)]
pub struct Updater {
    db: DbPool,
    cfg: watch::Receiver<AppConfig>,
    http: reqwest::Client,
    data_dir: PathBuf,
    /// `latest.json` 裡要找的鍵；`None` 代表這台機器認不出平台，永遠不會有可用更新
    platform: Option<String>,
    busy: Arc<AtomicBool>,
    last: Arc<std::sync::RwLock<Option<UpdateInfo>>>,
}

fn newer(latest: &str, current: &str) -> bool {
    match (semver::Version::parse(latest.trim_start_matches('v')), semver::Version::parse(current)) {
        (Ok(l), Ok(c)) => l > c,
        _ => latest != current,
    }
}

impl Updater {
    pub fn new(db: DbPool, cfg: watch::Receiver<AppConfig>, data_dir: &Path) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(format!("cix3752i-sorter/{CURRENT_VERSION}"))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client");
        let platform = headless_platform_key();
        match &platform {
            Some(p) => tracing::info!(platform = %p, "自動更新平台鍵"),
            None => tracing::warn!("認不出這台機器的平台（/etc/os-release 缺 ID／VERSION_ID），自動更新只會顯示新版、無法安裝"),
        }
        Self { db, cfg, http, data_dir: data_dir.to_path_buf(), platform, busy: Arc::new(AtomicBool::new(false)), last: Arc::new(std::sync::RwLock::new(None)) }
    }

    pub fn last_check(&self) -> Option<UpdateInfo> {
        self.last.read().unwrap().clone()
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Relaxed)
    }

    /// 啟動後 10 秒查一次，之後依設定間隔定期查；有新版就推 `update-available` 給前端
    pub fn start_background(&self, cancel: CancellationToken) {
        let me = self.clone();
        tokio::spawn(async move {
            tokio::select! { _ = cancel.cancelled() => return, _ = tokio::time::sleep(Duration::from_secs(10)) => {} }
            loop {
                if me.cfg.borrow().update.enabled {
                    if let Ok(info) = me.check().await {
                        if info.available {
                            event_bus::emit("update-available", &info);
                        }
                    }
                }
                let mins = me.cfg.borrow().update.check_interval_min.max(5);
                tokio::select! { _ = cancel.cancelled() => return, _ = tokio::time::sleep(Duration::from_secs(mins * 60)) => {} }
            }
        });
    }

    pub async fn check(&self) -> anyhow::Result<UpdateInfo> {
        let endpoint = self.cfg.borrow().update.endpoint.clone();
        let resp = self.http.get(&endpoint).timeout(Duration::from_secs(15)).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("更新伺服器回應 {}", resp.status());
        }
        let latest: LatestJson = resp.json().await?;
        let asset = self.platform.as_deref().and_then(|p| latest.platforms.get(p));
        let info = UpdateInfo {
            current: CURRENT_VERSION.into(),
            latest: latest.version.trim_start_matches('v').to_string(),
            available: asset.is_some() && newer(&latest.version, CURRENT_VERSION),
            notes: latest.notes.clone(),
            pub_date: latest.pub_date.clone(),
            url: asset.map(|a| a.url.clone()),
            sha256: asset.map(|a| a.sha256.clone()),
            checked_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        };
        *self.last.write().unwrap() = Some(info.clone());
        Ok(info)
    }

    /// 下載並安裝最近一次檢查到的版本
    pub async fn download_and_install(&self) -> anyhow::Result<()> {
        let info = self.last_check().ok_or_else(|| anyhow::anyhow!("請先檢查更新"))?;
        if !info.available {
            anyhow::bail!("目前已是最新版");
        }
        let (url, sha) = (info.url.clone().unwrap(), info.sha256.clone().unwrap_or_default());
        if self.busy.swap(true, Ordering::SeqCst) {
            anyhow::bail!("更新進行中");
        }
        let r = self.download_install_inner(&url, &sha, &info.latest).await;
        self.busy.store(false, Ordering::SeqCst);
        r
    }

    async fn download_install_inner(&self, url: &str, sha256: &str, version: &str) -> anyhow::Result<()> {
        let dir = self.data_dir.join("update");
        tokio::fs::create_dir_all(&dir).await?;
        let file = dir.join(format!("cix3752i-sorter_{version}.tar.gz"));
        progress("downloading", 0, None);

        let resp = self.http.get(url).timeout(Duration::from_secs(600)).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("下載失敗：HTTP {}", resp.status());
        }
        let total = resp.content_length();
        let mut out = tokio::fs::File::create(&file).await?;
        let mut hasher = Sha256::new();
        let mut done: u64 = 0;
        let mut last_pct = 0;
        let mut stream = resp.bytes_stream();
        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            hasher.update(&chunk);
            out.write_all(&chunk).await?;
            done += chunk.len() as u64;
            if let Some(t) = total.filter(|t| *t > 0) {
                let pct = (done * 100 / t) as u8;
                if pct != last_pct {
                    last_pct = pct;
                    progress("downloading", pct, Some(done));
                }
            }
        }
        out.flush().await?;
        drop(out);
        let digest = hex::encode(hasher.finalize());
        if !sha256.is_empty() && !digest.eq_ignore_ascii_case(sha256) {
            let _ = tokio::fs::remove_file(&file).await;
            anyhow::bail!("下載檔案校驗失敗（SHA-256 不符），已丟棄");
        }
        self.install_archive(&file, version).await
    }

    /// 網頁上傳的 tar.gz：直接安裝（沒有 sha 可比，靠設定密碼把關）
    pub async fn install_uploaded(&self, bytes: &[u8]) -> anyhow::Result<()> {
        if self.busy.swap(true, Ordering::SeqCst) {
            anyhow::bail!("更新進行中");
        }
        let r = async {
            let dir = self.data_dir.join("update");
            tokio::fs::create_dir_all(&dir).await?;
            let file = dir.join("uploaded.tar.gz");
            tokio::fs::write(&file, bytes).await?;
            self.install_archive(&file, "uploaded").await
        }
        .await;
        self.busy.store(false, Ordering::SeqCst);
        r
    }

    async fn install_archive(&self, archive: &Path, version: &str) -> anyhow::Result<()> {
        progress("installing", 100, None);
        let exe = std::env::current_exe()?;
        let archive = archive.to_path_buf();
        let exe2 = exe.clone();
        let new_version = tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
            let f = std::fs::File::open(&archive)?;
            let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
            let tmp = exe2.with_extension("new");
            let mut found = false;
            for entry in ar.entries()? {
                let mut e = entry?;
                let path = e.path()?.to_path_buf();
                if path.file_name().and_then(|n| n.to_str()) == Some("sorter") {
                    let mut out = std::fs::File::create(&tmp)?;
                    std::io::copy(&mut e, &mut out)?;
                    found = true;
                    break;
                }
            }
            if !found {
                anyhow::bail!("壓縮檔裡沒有 sorter 執行檔");
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
            }
            // 新檔先跑 --version 確認能執行，才蓋掉正在跑的
            let out = std::process::Command::new(&tmp).arg("--version").output()?;
            if !out.status.success() {
                let _ = std::fs::remove_file(&tmp);
                anyhow::bail!("新版執行檔無法啟動：{}", String::from_utf8_lossy(&out.stderr));
            }
            let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
            std::fs::rename(&tmp, &exe2)?;
            let _ = std::fs::remove_file(&archive);
            Ok(ver)
        })
        .await??;

        event_log::log(&self.db, Level::Warn, "server", "update", format!("已安裝 {new_version}（下載版本 {version}），1 秒後重啟"));
        event_bus::emit("update-installed", serde_json::json!({ "version": new_version }));
        tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(1000)).await;
            tracing::warn!("更新完成，結束行程交給 supervisor／systemd 重啟");
            std::process::exit(0);
        });
        Ok(())
    }
}

fn progress(stage: &str, percent: u8, bytes: Option<u64>) {
    event_bus::emit("update-progress", serde_json::json!({ "stage": stage, "percent": percent, "bytes": bytes }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 版本比較() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("v1.0.0", "0.9.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.0.9", "0.1.0"));
    }

    #[test]
    fn os_release_解析() {
        let focal = "NAME=\"Ubuntu\"\nVERSION=\"20.04.6 LTS (Focal Fossa)\"\nID=ubuntu\nID_LIKE=debian\nVERSION_ID=\"20.04\"\n";
        assert_eq!(linux_distro(focal).as_deref(), Some("ubuntu-20.04"));
        assert_eq!(linux_distro("ID=debian\nVERSION_ID=\"12\"\n").as_deref(), Some("debian-12"));
        // 滾動發行版沒有 VERSION_ID → 認不出來，不能亂配一份 .deb
        assert_eq!(linux_distro("ID=arch\n"), None);
    }

    #[test]
    fn latest_json_解析() {
        // 同一份 latest.json 同時給 Tauri updater（.deb，帶 signature）與 headless（tar.gz，帶 sha256）用，
        // 桌面那筆沒有 sha256 也不能讓整份解析失敗
        let j: LatestJson = serde_json::from_str(r#"{"version":"0.2.0","notes":"修正","pub_date":"2026-09-14T00:00:00Z","platforms":{
            "linux-x86_64-ubuntu-20.04":{"url":"https://x/a.deb","signature":"sig"},
            "linux-x86_64-ubuntu-20.04-headless":{"url":"https://x/a.tar.gz","signature":"sig","sha256":"ab"}}}"#).unwrap();
        assert_eq!(j.platforms["linux-x86_64-ubuntu-20.04-headless"].sha256, "ab");
        assert_eq!(j.platforms["linux-x86_64-ubuntu-20.04"].sha256, "");
    }
}
