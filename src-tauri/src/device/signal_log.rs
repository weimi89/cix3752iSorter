//! 裝置原始訊號留檔：皮帶／分揀機／相機收到與送出的每一行，逐日一檔寫在 `data/logs/signals-YYYY-MM-DD.log`。
//!
//! 這是事後查現場問題的唯一完整證據（`parcel_events` 只有綁到包裹的訊號；孤兒訊號、輸入點、
//! 光電查詢回覆、送出的指令都不在那裡）。舊系統的 `cmd.log` 就是靠這個把協定逆推出來的。
//! 停止中每個週期都會送的 `~k-1`，同一裝置 60 秒只記一次，檔案才不會被洗版。
//!
//! 寫檔在獨立執行緒，裝置 task 只丟訊息不等磁碟；沒 `init` 時 `record` 是空操作（單元測試）。

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

const FILE_PREFIX: &str = "signals-";
const STOPPED_SUPPRESS_MS: i64 = 60_000;

#[derive(Clone, Copy)]
pub enum Dir {
    /// 裝置 → 本程式
    In,
    /// 本程式 → 裝置
    Out,
    /// 連線／斷線等狀態
    Info,
}

struct Entry {
    ts_ms: i64,
    device: &'static str,
    dir: Dir,
    text: String,
}

static TX: OnceLock<Sender<Entry>> = OnceLock::new();

/// 啟動寫檔執行緒；重複呼叫無效
pub fn init(dir: PathBuf, keep_days: u32) {
    if TX.get().is_some() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::error!(dir = %dir.display(), "訊號日誌目錄建立失敗，訊號不留檔: {e}");
        return;
    }
    let (tx, rx) = mpsc::channel::<Entry>();
    if TX.set(tx).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("signal-log".into())
        .spawn(move || writer(dir, keep_days, rx))
        .expect("signal-log thread");
}

pub fn record(device: &'static str, dir: Dir, text: &str) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Entry { ts_ms: crate::db::now_ms(), device, dir, text: text.to_string() });
    }
}

fn writer(dir: PathBuf, keep_days: u32, rx: mpsc::Receiver<Entry>) {
    let mut current_day = String::new();
    let mut file: Option<BufWriter<File>> = None;
    let mut last_stopped: HashMap<&'static str, i64> = HashMap::new();
    let mut dirty = false;
    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(e) => {
                // `~k-1`：停止中的週期訊號，60 秒記一次就夠
                if e.text.starts_with("~k-1") {
                    let last = last_stopped.get(e.device).copied().unwrap_or(i64::MIN / 2);
                    if e.ts_ms - last < STOPPED_SUPPRESS_MS {
                        continue;
                    }
                    last_stopped.insert(e.device, e.ts_ms);
                }
                let local = chrono::DateTime::from_timestamp_millis(e.ts_ms).map(|t| t.with_timezone(&chrono::Local));
                let day = local.map(|t| t.format("%Y-%m-%d").to_string()).unwrap_or_default();
                if day != current_day {
                    if let Some(mut f) = file.take() {
                        let _ = f.flush();
                    }
                    current_day = day.clone();
                    file = open_day(&dir, &day);
                    prune(&dir, keep_days);
                }
                if let Some(f) = file.as_mut() {
                    let clock = local.map(|t| t.format("%H:%M:%S%.3f").to_string()).unwrap_or_default();
                    let arrow = match e.dir {
                        Dir::In => "<",
                        Dir::Out => ">",
                        Dir::Info => "*",
                    };
                    let _ = writeln!(f, "{clock} {:<6} {arrow} {}", e.device, e.text);
                    dirty = true;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if dirty {
                    if let Some(f) = file.as_mut() {
                        let _ = f.flush();
                    }
                    dirty = false;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                if let Some(mut f) = file.take() {
                    let _ = f.flush();
                }
                return;
            }
        }
    }
}

fn open_day(dir: &Path, day: &str) -> Option<BufWriter<File>> {
    let path = dir.join(format!("{FILE_PREFIX}{day}.log"));
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => Some(BufWriter::new(f)),
        Err(e) => {
            tracing::error!(path = %path.display(), "訊號日誌開檔失敗: {e}");
            None
        }
    }
}

/// 刪掉超過保留天數的檔（0 = 不刪）；檔名裡的日期就是依據，不看 mtime
fn prune(dir: &Path, keep_days: u32) {
    if keep_days == 0 {
        return;
    }
    let cutoff = (chrono::Local::now() - chrono::Duration::days(keep_days as i64)).format("%Y-%m-%d").to_string();
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(day) = name.strip_prefix(FILE_PREFIX).and_then(|s| s.strip_suffix(".log")) else { continue };
        if day < cutoff.as_str() {
            if let Err(e) = std::fs::remove_file(entry.path()) {
                tracing::warn!(file = name, "舊訊號日誌刪除失敗: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 依檔名日期清舊檔() {
        let dir = std::env::temp_dir().join(format!("sig-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = (chrono::Local::now() - chrono::Duration::days(20)).format("%Y-%m-%d").to_string();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        for d in [&old, &today] {
            std::fs::write(dir.join(format!("{FILE_PREFIX}{d}.log")), "x").unwrap();
        }
        std::fs::write(dir.join("other.log"), "x").unwrap();
        prune(&dir, 15);
        assert!(!dir.join(format!("{FILE_PREFIX}{old}.log")).exists());
        assert!(dir.join(format!("{FILE_PREFIX}{today}.log")).exists());
        assert!(dir.join("other.log").exists(), "非訊號日誌不動");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
