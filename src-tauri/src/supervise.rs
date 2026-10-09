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
    tokio::spawn(async move {
        let Err(e) = task.await else { return };
        if !e.is_panic() {
            return;
        }
        let reason = crate::log::panic_text(e.into_panic().as_ref());
        let message = format!("背景工作「{name}」異常停止：{reason}。這項功能已經停止運作，請重新啟動程式");
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
        assert!(got["message"].as_str().unwrap().contains("故意崩潰"), "要帶崩潰原因：{got}");
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
}
