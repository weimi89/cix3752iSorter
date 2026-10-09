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

/// 網頁查詢用的唯讀連線池最多幾條
pub const READ_POOL_SIZE: u32 = 4;

/// 網頁查詢（統計、清單、匯出、異常存證…）用的唯讀連線池，跟狀態機寫入的池分開。
///
/// 共用一個池的話，同時幾個長查詢就把連線全部占住，狀態機的包裹寫入只能排隊，
/// 寫入佇列滿了就丟。分開後查詢再多也只會排在這 4 條裡，寫入池的連線永遠留給寫入。WAL 下讀與寫可以同時進行。
/// 要在 `init` 跑完 migration 之後才建（`_write` 只是用來表達這個先後）
pub async fn init_read(data_dir: &Path, _write: &DbPool) -> anyhow::Result<DbPool> {
    let options = SqliteConnectOptions::new()
        .filename(data_dir.join("sorter.sqlite"))
        .read_only(true)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(5));
    Ok(SqlitePoolOptions::new().max_connections(READ_POOL_SIZE).connect_with(options).await?)
}

/// SQLITE_BUSY（5）與 SQLITE_LOCKED（6）家族，含延伸碼（例如 517 = BUSY_SNAPSHOT）
pub fn is_busy(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Database(d) => d.code().and_then(|c| c.parse::<i32>().ok()).is_some_and(|c| matches!(c & 0xff, 5 | 6)),
        _ => false,
    }
}

/// 資料庫忙碌時的重試次數（間隔 0.2／0.6／1.8／5.4 秒，另加每次的 busy_timeout）
const BUSY_RETRIES: u32 = 4;
/// 清理過期資料一次刪多少列
pub const PURGE_BATCH: i64 = 500;
/// 每批之間讓出寫入鎖的時間：SQLite 等待中的寫入會以逐漸拉長的間隔重試，
/// 一批刪完立刻再搶鎖的話，排隊的狀態機寫入可能一直插不進來
pub const PURGE_PAUSE: std::time::Duration = std::time::Duration::from_millis(20);

/// 分批刪除 `table` 中符合 `cond`（帶一個 `?`）的列，回傳總筆數。
///
/// SQLite 只有一個寫入者，一道 DELETE 刪幾萬列（連帶外鍵級聯）期間其他寫入全部排隊，
/// 超過 busy_timeout（5 秒）就失敗——保留天數改短或第一次開清理時，一次要刪的就是幾十天的資料。
/// `table`、`cond` 一律傳字面值
pub async fn delete_in_batches<T>(db: &DbPool, table: &'static str, cond: &'static str, arg: T) -> Result<u64, sqlx::Error>
where
    T: for<'q> sqlx::Encode<'q, Sqlite> + sqlx::Type<Sqlite> + Clone + Send + Sync + 'static,
{
    let sql = format!("DELETE FROM {table} WHERE rowid IN (SELECT rowid FROM {table} WHERE {cond} LIMIT {PURGE_BATCH})");
    let mut total = 0;
    loop {
        let n = sqlx::query(sqlx::AssertSqlSafe(sql.clone())).bind(arg.clone()).execute(db).await?.rows_affected();
        total += n;
        if (n as i64) < PURGE_BATCH {
            return Ok(total);
        }
        tokio::time::sleep(PURGE_PAUSE).await;
    }
}


/// 只在資料庫忙碌（等滿 busy_timeout 仍拿不到寫鎖）時退避重試，其他錯誤照常回傳。
/// 給「失敗就沒有人會再做一次」的寫入用（回報入列、照片存檔）
pub async fn retry_busy<T, F, Fut>(what: &str, mut op: F) -> Result<T, sqlx::Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, sqlx::Error>>,
{
    let mut delay = std::time::Duration::from_millis(200);
    let mut attempt = 0;
    loop {
        match op().await {
            Err(e) if attempt < BUSY_RETRIES && is_busy(&e) => {
                attempt += 1;
                tracing::warn!(what, attempt, "資料庫忙碌，稍後重試: {e}");
                tokio::time::sleep(delay).await;
                delay *= 3;
            }
            r => return r,
        }
    }
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

/// 保留天數換算的截止日（本機日期 `YYYY-MM-DD`），早於這天的資料可清。
/// 天數大到早於日期能表示的範圍（設定檔手改出極大值）時回 `None`＝沒有那麼舊的資料；
/// 直接相減會讓清理工作恐慌，從此再也不清
pub fn retention_cutoff_day(days: u32) -> Option<String> {
    let delta = chrono::Duration::try_days(days as i64)?;
    chrono::Local::now().checked_sub_signed(delta).map(|t| t.format("%Y-%m-%d").to_string())
}

/// 本機時間字串 `YYYY-MM-DD HH:MM:SS.mmm`（歷史查詢與匯出用）
pub fn local_ts(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn 網頁查詢把讀取池用滿時_寫入仍要立刻拿得到連線() {
        // 統計、匯出這類長查詢同時好幾個時會把連線全部占住；跟寫入共用一個池的話，
        // 狀態機的包裹寫入只能排隊，寫入佇列滿了就丟
        let dir = std::env::temp_dir().join(format!("pools-{}", ulid::Ulid::generate()));
        let write = init(&dir).await.unwrap();
        let read = init_read(&dir, &write).await.unwrap();
        let mut held = Vec::new();
        while let Ok(Ok(c)) = tokio::time::timeout(std::time::Duration::from_millis(200), read.acquire()).await {
            held.push(c);
            if held.len() > 64 {
                break;
            }
        }
        assert!(!held.is_empty() && held.len() <= 64, "讀取池要有上限，實際借到 {} 條", held.len());
        let t = std::time::Instant::now();
        let r = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            sqlx::query("INSERT INTO event_log (level, category, action, message, created_at) VALUES ('info', 't', 't', 't', '2026-01-01')").execute(&write),
        )
        .await;
        assert!(matches!(r, Ok(Ok(_))), "讀取池用滿 {} 條時，寫入等了 {:?} 還拿不到連線", held.len(), t.elapsed());
    }

    #[tokio::test]
    async fn 讀取池不能寫入() {
        let dir = std::env::temp_dir().join(format!("pools-{}", ulid::Ulid::generate()));
        let write = init(&dir).await.unwrap();
        let read = init_read(&dir, &write).await.unwrap();
        let r = sqlx::query("INSERT INTO event_log (level, category, action, message, created_at) VALUES ('info', 't', 't', 't', '2026-01-01')").execute(&read).await;
        assert!(r.is_err(), "網頁查詢用的池只能讀");
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM parcels").fetch_one(&read).await.unwrap();
        assert_eq!(n, 0, "讀得到寫入池建好的表");
    }
}
