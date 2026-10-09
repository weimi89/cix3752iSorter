//! 原子寫檔：寫同目錄隱藏臨時檔 → `rename` 覆蓋目標。
//!
//! 直接 `fs::write(target)` 會先 truncate 再寫，空窗中被讀到就是截斷檔
//! （設定檔會變成解析失敗、面單點陣檔會印出半張）。rename 在同一檔案系統是原子的，
//! 讀取端永遠看到完整的舊檔或新檔。寫入失敗時清掉臨時檔再回錯，不留孤兒。
//!
//! [`Durability::Synced`]：改名前先把內容落盤、改名後再讓目錄落盤。只靠改名的話，斷電後可能留下
//! 「改名生效但內容還沒寫到磁碟」的空檔——設定檔變空程式就起不來，列印檔變空重開後補印的就是空白面單。
//! 每次多兩次強制落盤，量大的檔案（照片）不要用。

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn sibling_tmp(target: &Path) -> PathBuf {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let name = target.file_name().and_then(|s| s.to_str()).unwrap_or("file");
    target.with_file_name(format!(".{name}.part.{}.{}", std::process::id(), seq))
}

/// 寫完要不要強制落盤
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Durability {
    /// 設定檔、列印檔：斷電後變成空檔的代價是程式起不來或印出空白面單
    Synced,
    /// 照片：一天幾千張，斷電遺失最後幾張可以接受，不值得每張多兩次強制落盤搶磁碟
    Unsynced,
}

pub async fn write_async(target: &Path, data: &[u8], durability: Durability) -> io::Result<()> {
    let target = target.to_path_buf();
    let data = data.to_vec();
    tokio::task::spawn_blocking(move || write_blocking(&target, &data, durability)).await.map_err(io::Error::other)?
}

fn write_blocking(target: &Path, data: &[u8], durability: Durability) -> io::Result<()> {
    let tmp = sibling_tmp(target);
    if let Err(e) = write_file(&tmp, data, durability) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if durability == Durability::Synced {
        sync_parent(target);
    }
    Ok(())
}

fn write_file(path: &Path, data: &[u8], durability: Durability) -> io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    f.write_all(data)?;
    if durability == Durability::Synced {
        f.sync_all()?;
    }
    Ok(())
}

/// 目標檔所在的目錄；只寫檔名（`config.toml`）時 `parent()` 是空路徑，那就是目前目錄
fn parent_dir(target: &Path) -> Option<&Path> {
    target.parent().map(|p| if p.as_os_str().is_empty() { Path::new(".") } else { p })
}

/// 讓改名這件事本身落盤。內容已經落盤、改名也已生效，這一步失敗只代表斷電時可能還看到舊檔，記下來即可
fn sync_parent(target: &Path) {
    #[cfg(unix)]
    if let Some(dir) = parent_dir(target)
        && let Err(e) = std::fs::File::open(dir).and_then(|d| d.sync_all())
    {
        tracing::warn!(dir = %dir.display(), "目錄落盤失敗: {e}");
    }
    #[cfg(not(unix))]
    let _ = target;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 要落盤的目錄_只寫檔名時是目前目錄_不可略過() {
        // `--config config.toml` 這種寫法很常見；略過的話改名沒落盤，斷電後可能回到舊設定
        assert_eq!(parent_dir(Path::new("config.toml")), Some(Path::new(".")));
        assert_eq!(parent_dir(Path::new("data/print/1.tspl")), Some(Path::new("data/print")));
        assert_eq!(parent_dir(Path::new("/etc/sorter/config.toml")), Some(Path::new("/etc/sorter")));
    }

    #[tokio::test]
    async fn 寫入後目錄內不留臨時檔() {
        let dir = std::env::temp_dir().join(format!("cix-fs-atomic-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let target = dir.join("a.txt");
        write_async(&target, b"hello", Durability::Synced).await.unwrap();
        write_async(&target, b"world", Durability::Unsynced).await.unwrap();
        assert_eq!(tokio::fs::read(&target).await.unwrap(), b"world");
        let mut rd = tokio::fs::read_dir(&dir).await.unwrap();
        let mut names = vec![];
        while let Some(e) = rd.next_entry().await.unwrap() {
            names.push(e.file_name().to_string_lossy().into_owned());
        }
        assert_eq!(names, vec!["a.txt"]);
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    /// 量測用（`--ignored --nocapture`）：照片大小的檔案，落盤與不落盤各寫一批的耗時
    #[tokio::test]
    #[ignore]
    async fn 照片大小的檔案_落盤與不落盤的寫入耗時() {
        let dir = std::env::temp_dir().join(format!("cix-fs-bench-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let data = vec![0x5au8; 620 * 1024];
        for d in [Durability::Synced, Durability::Unsynced, Durability::Synced, Durability::Unsynced] {
            let t = std::time::Instant::now();
            for i in 0..200 {
                write_async(&dir.join(format!("{i}.jpg")), &data, d).await.unwrap();
            }
            println!("{d:?}: 200 張共 {} ms", t.elapsed().as_millis());
        }
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
