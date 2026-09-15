//! USB 印表機定位：設定寫的是 USB 匯流排埠位（如 `1-3.1`，插在哪個孔就是哪個），
//! 開機或熱插拔後 `/dev/usb/lpN` 的 N 會變，每次寫入前都重新對一次。
//!
//! 對法：`/sys/class/usbmisc/lpN/device` 的真實路徑倒數第二段就是埠位。

use std::path::{Path, PathBuf};

pub const SYS_USBMISC: &str = "/sys/class/usbmisc";

/// 在 `sys_root`（正式為 `/sys/class/usbmisc`）下找埠位對應的裝置檔
pub fn find_printer_in(sys_root: &Path, port: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(sys_root).ok()?;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("lp") {
            continue;
        }
        let Ok(real) = std::fs::canonicalize(e.path().join("device")) else { continue };
        let segs: Vec<String> = real.iter().map(|s| s.to_string_lossy().into_owned()).collect();
        if segs.len() < 3 {
            continue;
        }
        if segs[segs.len() - 2] == port {
            return Some(PathBuf::from("/dev/usb").join(name));
        }
    }
    None
}

pub fn find_printer(port: &str) -> Option<PathBuf> {
    find_printer_in(Path::new(SYS_USBMISC), port)
}

/// 列出目前接著的所有印表機（埠位 → 裝置檔），設定頁用
pub fn list_printers_in(sys_root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(sys_root) else { return out };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("lp") {
            continue;
        }
        let Ok(real) = std::fs::canonicalize(e.path().join("device")) else { continue };
        let segs: Vec<String> = real.iter().map(|s| s.to_string_lossy().into_owned()).collect();
        if segs.len() >= 3 {
            out.push((segs[segs.len() - 2].clone(), PathBuf::from("/dev/usb").join(name)));
        }
    }
    out.sort();
    out
}

pub fn list_printers() -> Vec<(String, PathBuf)> {
    list_printers_in(Path::new(SYS_USBMISC))
}

/// 同步寫入裝置檔（呼叫端放 `spawn_blocking`）。先以空寫入探測節點是否還活著。
pub fn write_device(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
    f.write_all(data)?;
    f.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 模擬 sysfs：`<root>/lp3/device` 是指向 `<root>/devices/pci/usb1/1-3/1-3.1/1-3.1:1.0` 的符號連結
    fn fake_sysfs() -> PathBuf {
        let root = std::env::temp_dir().join(format!("cix-usb-{}-{}", std::process::id(), crate::db::now_ms()));
        let dev = root.join("devices/pci/usb1/1-3/1-3.1/1-3.1:1.0");
        std::fs::create_dir_all(&dev).unwrap();
        let dev2 = root.join("devices/pci/usb1/1-8/1-8.4/1-8.4:1.0");
        std::fs::create_dir_all(&dev2).unwrap();
        std::fs::create_dir_all(root.join("lp3")).unwrap();
        std::fs::create_dir_all(root.join("lp0")).unwrap();
        std::fs::create_dir_all(root.join("hiddev0")).unwrap();
        std::os::unix::fs::symlink(&dev, root.join("lp3/device")).unwrap();
        std::os::unix::fs::symlink(&dev2, root.join("lp0/device")).unwrap();
        root
    }

    #[test]
    fn 依埠位找到裝置檔() {
        let root = fake_sysfs();
        assert_eq!(find_printer_in(&root, "1-3.1"), Some(PathBuf::from("/dev/usb/lp3")));
        assert_eq!(find_printer_in(&root, "1-8.4"), Some(PathBuf::from("/dev/usb/lp0")));
        assert_eq!(find_printer_in(&root, "1-3.2"), None);
        let list = list_printers_in(&root);
        assert_eq!(list, vec![("1-3.1".to_string(), PathBuf::from("/dev/usb/lp3")), ("1-8.4".to_string(), PathBuf::from("/dev/usb/lp0"))]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sysfs_不存在時回空() {
        assert_eq!(find_printer_in(Path::new("/nonexistent/usbmisc"), "1-3.1"), None);
        assert!(list_printers_in(Path::new("/nonexistent/usbmisc")).is_empty());
    }
}
