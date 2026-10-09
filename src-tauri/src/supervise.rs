//! 常駐背景工作的監督。
//!
//! tokio 的工作崩潰時只結束那個工作，程式其他部分照跑：狀態機崩潰後皮帶照轉、畫面停在最後狀態，
//! 印表機的佇列工作崩潰後那台印表機從此不印，都沒有任何人知道。從開機跑到關機的工作一律用這裡啟動，
//! 崩潰時寫錯誤日誌、推一則錯誤訊息到畫面，並寫進事件紀錄表（沒人看畫面的時段，事後在事件紀錄頁查得到）。每個連線、每個請求各自開的短暫工作不必走這裡
//! （崩潰只影響那一筆，`log::install_panic_hook` 會留下紀錄）。

use std::future::Future;
use std::sync::OnceLock;

use crate::db::DbPool;

static EVENT_DB: OnceLock<DbPool> = OnceLock::new();

/// 已異常停止的常駐工作。功能要到程式重開才會恢復，畫面依這份清單一直顯示警示，
/// 不能只靠崩潰當下推一次訊息（那時沒人開著畫面、或之後重新整理就看不到了）
#[derive(Clone, Debug, serde::Serialize)]
pub struct StoppedTask {
    pub name: String,
    pub reason: String,
    pub at: String,
}

static STOPPED: std::sync::Mutex<Vec<StoppedTask>> = std::sync::Mutex::new(Vec::new());

pub fn stopped_tasks() -> Vec<StoppedTask> {
    STOPPED.lock().map(|v| v.clone()).unwrap_or_default()
}

/// 啟動時登記一次。登記前（或測試裡沒登記）崩潰的只寫日誌、推畫面，不寫表
pub fn log_panics_to(db: DbPool) {
    let _ = EVENT_DB.set(db);
}

/// 啟動常駐工作；崩潰時通知。`name` 用現場看得懂的說法（「狀態機」「列印佇列 1-8.1」）
pub fn spawn<F>(name: impl Into<String>, fut: F)
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    watch(name.into(), tokio::spawn(fut));
}

/// 同 [`spawn`]，另外登記崩潰時的補救動作（例如狀態機崩潰時停皮帶）；正常結束不執行
pub fn spawn_or<F>(name: impl Into<String>, fut: F, on_panic: impl FnOnce() + Send + 'static)
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    watch_then(name.into(), tokio::spawn(fut), Some(Box::new(on_panic)));
}

/// 結束前必須做完的常駐工作（例如包裹寫入）：照樣受監督，另外登記在 `tasks`，
/// 結束流程用 [`drain`] 等它把手上的寫完
pub fn spawn_tracked<F>(name: impl Into<String>, tasks: &tokio_util::task::TaskTracker, fut: F)
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    watch(name.into(), tasks.spawn(fut));
}

fn watch<T: Send + 'static>(name: String, task: tokio::task::JoinHandle<T>) {
    watch_then(name, task, None);
}

fn watch_then<T: Send + 'static>(name: String, task: tokio::task::JoinHandle<T>, on_panic: Option<Box<dyn FnOnce() + Send>>) {
    tokio::spawn(async move {
        let Err(e) = task.await else { return };
        if !e.is_panic() {
            return;
        }
        if let Some(f) = on_panic {
            f();
        }
        let reason = crate::log::panic_text(e.into_panic().as_ref());
        if let Ok(mut v) = STOPPED.lock() {
            v.push(StoppedTask { name: name.clone(), reason: reason.clone(), at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string() });
        }
        // 錯誤原文（多半是英文工程訊息）只進日誌檔，畫面與事件紀錄只寫現場看得懂的話
        tracing::error!(task = %name, %reason, "背景工作異常停止");
        let message = format!("背景工作「{name}」異常停止，這項功能已經停止運作，請確認現場安全後重新啟動程式");
        if let Some(db) = EVENT_DB.get() {
            crate::event_log::log(db, crate::event_log::Level::Error, "system", "task_panic", message);
            return;
        }
        tracing::error!(task = %name, "{message}");
        crate::event_bus::emit(
            "system-message",
            serde_json::json!({
                "level": "error",
                "category": "system",
                "action": "task_panic",
                "message": message,
                "created_at": chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
            }),
        );
    });
}

/// 結束流程：等 `tasks` 裡的工作做完，最多 `limit`。回 `false` 代表時間到還有沒做完的。
/// 呼叫前要先發出結束通知，讓這些工作的資料來源停下來，否則會一直等到時限
pub async fn drain(tasks: &tokio_util::task::TaskTracker, limit: std::time::Duration) -> bool {
    tasks.close();
    tokio::time::timeout(limit, tasks.wait()).await.is_ok()
}

/// 結束流程等待寫入的時限。正常幾百毫秒就寫完、不會等滿；要涵蓋資料庫忙碌時的重試
/// （每次等寫鎖 5 秒、間隔 0.2／0.6 秒），又要短於切換腳本強制結束前的 20 秒（`deploy/switch.sh`）
pub const DRAIN_LIMIT: std::time::Duration = std::time::Duration::from_secs(15);

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn 常駐工作崩潰_要推錯誤訊息給畫面_並帶工作名稱() {
        let mut rx = crate::event_bus::subscribe();
        let marker = format!("測試工作-{}", ulid::Ulid::generate());
        spawn(marker.clone(), async { panic!("故意崩潰") });
        let got = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let ev = rx.recv().await.unwrap();
                if ev.event == "system-message" && ev.payload["message"].as_str().is_some_and(|m| m.contains(marker.as_str())) {
                    return ev.payload;
                }
            }
        })
        .await
        .expect("兩秒內要收到崩潰通知");
        assert_eq!(got["level"], "error");
        // 畫面與事件紀錄給現場看：程式內部的錯誤原文（多半是英文工程訊息）只留在日誌檔與已停止清單
        assert!(!got["message"].as_str().unwrap().contains("故意崩潰"), "不可把錯誤原文給現場看：{got}");
    }

    #[tokio::test]
    async fn 常駐工作正常結束_不發通知() {
        let mut rx = crate::event_bus::subscribe();
        let marker = format!("正常工作-{}", ulid::Ulid::generate());
        spawn(marker.clone(), async {});
        let r = tokio::time::timeout(std::time::Duration::from_millis(300), async {
            loop {
                let ev = rx.recv().await.unwrap();
                if ev.payload["message"].as_str().is_some_and(|m| m.contains(marker.as_str())) {
                    return;
                }
            }
        })
        .await;
        assert!(r.is_err(), "正常結束不該發通知");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn 常駐工作崩潰_要寫進事件紀錄表_沒人看畫面時事後也查得到() {
        let dir = std::env::temp_dir().join(format!("sup-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        log_panics_to(crate::db::init(&dir).await.unwrap());
        // 登記是全程式一次，別的測試先登記的話就查那一個
        let db = EVENT_DB.get().unwrap().clone();
        let marker = format!("測試工作-{}", ulid::Ulid::generate());
        spawn(marker.clone(), async { panic!("故意崩潰") });
        let mut n = 0i64;
        for _ in 0..40 {
            n = sqlx::query_scalar("SELECT count(*) FROM event_log WHERE level = 'error' AND action = 'task_panic' AND message LIKE '%' || ? || '%'")
                .bind(&marker).fetch_one(&db).await.unwrap();
            if n > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn 常駐工作崩潰時_要執行登記的補救動作() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        spawn_or(format!("測試工作-{}", ulid::Ulid::generate()), async { panic!("故意崩潰") }, move || {
            let _ = tx.send(());
        });
        assert!(tokio::time::timeout(std::time::Duration::from_secs(2), rx).await.is_ok_and(|r| r.is_ok()), "崩潰後兩秒內要執行補救動作");
    }

    #[tokio::test]
    async fn 常駐工作正常結束_不執行補救動作() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        spawn_or("正常結束的工作", async {}, move || {
            let _ = tx.send(());
        });
        let r = tokio::time::timeout(std::time::Duration::from_millis(300), rx).await;
        assert!(!matches!(r, Ok(Ok(()))), "正常結束不可執行補救動作");
    }

    #[tokio::test]
    async fn 常駐工作崩潰後_狀態查詢列得出來_直到程式重開() {
        let marker = format!("測試工作-{}", ulid::Ulid::generate());
        spawn(marker.clone(), async { panic!("故意崩潰") });
        let mut found = None;
        for _ in 0..40 {
            found = stopped_tasks().into_iter().find(|t| t.name == marker);
            if found.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let t = found.expect("崩潰的工作要列在已停止清單");
        assert!(t.reason.contains("故意崩潰"));
    }
}
