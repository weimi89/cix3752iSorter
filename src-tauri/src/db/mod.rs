//! SQLite 連線池與 migration。

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
pub fn now_ms() -> i64 {
    chrono::Local::now().timestamp_millis()
}

/// 本機時間字串 `YYYY-MM-DD HH:MM:SS.mmm`（歷史查詢與匯出用）
pub fn local_ts(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        .unwrap_or_default()
}
