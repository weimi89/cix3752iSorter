//! SQLite 連線池與 migration。

pub mod abnormal_kind;
pub mod retention;

use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};

pub type DbPool = Pool<Sqlite>;

/// 開啟 `data_dir/sorter.sqlite`（WAL），執行編譯期內嵌的 migrations。
pub async fn init(data_dir: &Path) -> anyhow::Result<DbPool> {
    tokio::fs::create_dir_all(data_dir).await?;
    let db_path = data_dir.join("sorter.sqlite");

    let options = SqliteConnectOptions::new()
        .filename(&db_path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        // 分揀尖峰每秒好幾筆寫入；NORMAL 在 WAL 下只有斷電才可能丟最後幾筆，換來寫入不等 fsync
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    tracing::info!(path = %db_path.display(), "SQLite 初始化完成");
    Ok(pool)
}

/// 現在的 epoch 毫秒
/// 本地時區今天 00:00 的 epoch 毫秒（「今天的前科」這類查詢用）
pub fn today_start_ms() -> i64 {
    use chrono::TimeZone;
    let today = chrono::Local::now().date_naive();
    chrono::Local
        .from_local_datetime(&today.and_hms_opt(0, 0, 0).expect("00:00:00 必定合法"))
        .single()
        .map(|t| t.timestamp_millis())
        .unwrap_or_else(now_ms)
}

pub fn now_ms() -> i64 {
    chrono::Local::now().timestamp_millis()
}

/// 本機時間字串 `YYYY-MM-DD HH:MM:SS.mmm`（歷史查詢與匯出用）
pub fn local_ts(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        .unwrap_or_default()
}
