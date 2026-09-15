//! 自動更新只有一條路：Tauri updater 外掛讀 GitHub Release 的 `latest.json`、抓對應 distro 的 .deb 安裝。
//! 這裡只負責算出這台機器在 `latest.json` `platforms` 裡的鍵（`desktop.rs` 拿去當 updater 的 target）。

/// 這台機器在 `latest.json` `platforms` 裡對應的鍵，例如 `linux-x86_64-ubuntu-20.04`。
///
/// Linux 一定帶 distro：20.04 的 .deb 連的是自編 glib/webkit 棧、22.04+ 連系統套件，兩邊不能互換，
/// 發版時是各 distro 各出一份。distro 讀不出來（非 Ubuntu／Debian 系或 `/etc/os-release` 缺欄位）
/// 回 `None`，視為「沒有可用更新」，不會亂配一份 .deb。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_解析() {
        let focal = "NAME=\"Ubuntu\"\nVERSION=\"20.04.6 LTS (Focal Fossa)\"\nID=ubuntu\nID_LIKE=debian\nVERSION_ID=\"20.04\"\n";
        assert_eq!(linux_distro(focal).as_deref(), Some("ubuntu-20.04"));
        assert_eq!(linux_distro("ID=debian\nVERSION_ID=\"12\"\n").as_deref(), Some("debian-12"));
        // 滾動發行版沒有 VERSION_ID → 認不出來，不能亂配一份 .deb
        assert_eq!(linux_distro("ID=arch\n"), None);
    }
}
