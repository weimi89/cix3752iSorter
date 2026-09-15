//! 系統事件記錄：寫 `event_log` 表並推 `system-message` 給前端。
//!
//! fire-and-forget，不阻塞呼叫端（裝置迴圈裡也能安心呼叫）。

use crate::db::DbPool;
use crate::event_bus;

#[derive(Clone, Copy, Debug)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

pub fn log(db: &DbPool, level: Level, category: &'static str, action: &'static str, message: impl Into<String>) {
    let message = message.into();
    let created_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();

    match level {
        Level::Info => tracing::info!(category, action, "{message}"),
        Level::Warn => tracing::warn!(category, action, "{message}"),
        Level::Error => tracing::error!(category, action, "{message}"),
    }

    event_bus::emit(
        "system-message",
        serde_json::json!({
            "level": level.as_str(),
            "category": category,
            "action": action,
            "message": message,
            "created_at": created_at,
        }),
    );

    let db = db.clone();
    tokio::spawn(async move {
        if let Err(e) = sqlx::query(
            "INSERT INTO event_log (level, category, action, message, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(level.as_str())
        .bind(category)
        .bind(action)
        .bind(message)
        .bind(created_at)
        .execute(&db)
        .await
        {
            tracing::error!("event_log 寫入失敗: {e}");
        }
    });
}
