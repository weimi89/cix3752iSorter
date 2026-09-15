//! 原子寫檔：寫同目錄隱藏臨時檔 → `rename` 覆蓋目標。
//!
//! 直接 `fs::write(target)` 會先 truncate 再寫，空窗中被讀到就是截斷檔
//! （設定檔會變成解析失敗、面單點陣檔會印出半張）。rename 在同一檔案系統是原子的，
//! 讀取端永遠看到完整的舊檔或新檔。寫入失敗時清掉臨時檔再回錯，不留孤兒。

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn sibling_tmp(target: &Path) -> PathBuf {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let name = target.file_name().and_then(|s| s.to_str()).unwrap_or("file");
    target.with_file_name(format!(".{name}.part.{}.{}", std::process::id(), seq))
}

pub async fn write_async(target: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = sibling_tmp(target);
    if let Err(e) = tokio::fs::write(&tmp, data).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }
    if let Err(e) = tokio::fs::rename(&tmp, target).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }
    Ok(())
}

pub fn write_sync(target: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = sibling_tmp(target);
    if let Err(e) = std::fs::write(&tmp, data) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn 寫入後目錄內不留臨時檔() {
        let dir = std::env::temp_dir().join(format!("cix-fs-atomic-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let target = dir.join("a.txt");
        write_async(&target, b"hello").await.unwrap();
        write_async(&target, b"world").await.unwrap();
        assert_eq!(tokio::fs::read(&target).await.unwrap(), b"world");
        let mut rd = tokio::fs::read_dir(&dir).await.unwrap();
        let mut names = vec![];
        while let Some(e) = rd.next_entry().await.unwrap() {
            names.push(e.file_name().to_string_lossy().into_owned());
        }
        assert_eq!(names, vec!["a.txt"]);
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
