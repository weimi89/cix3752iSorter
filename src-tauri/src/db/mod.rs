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

    let migrator = sqlx::migrate!("./migrations");
    // 備份失敗（磁碟滿、目錄權限）只記錯誤照常升級：起不來會讓整條線停住，比不能降版嚴重
    if let Err(e) = backup_before_migrate(&pool, data_dir, &migrator).await {
        tracing::error!("套用資料表變更前的備份失敗，照常升級（這次升級後不能降版）: {e}");
    }
    migrator.run(&pool).await?;

    tracing::info!(path = %db_path.display(), "SQLite 初始化完成");
    Ok(pool)
}

/// 升級前的資料庫備份保留幾份
pub const BACKUPS_KEPT: usize = 3;

/// 有新的資料表變更要套用、而且已經有資料時，先把整個資料庫備份到 `data_dir/backups/`，只留最近幾份。
///
/// 變更套用後，舊版程式遇到不認得的變更紀錄會拒絕啟動（不能裝回舊版救急）；要降版只能把這份備份放回去。
/// 程式內建的自動更新不經過任何人工步驟，所以由程式自己在套用前備份。全新的資料庫沒東西可備，不做
pub async fn backup_before_migrate(pool: &DbPool, data_dir: &Path, migrator: &sqlx::migrate::Migrator) -> anyhow::Result<Option<std::path::PathBuf>> {
    let has_table: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations')").fetch_one(pool).await?;
    if !has_table {
        return Ok(None);
    }
    let applied: Vec<i64> = sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success").fetch_all(pool).await?;
    let pending: Vec<i64> = migrator.iter().filter(|m| m.migration_type.is_up_migration() && !applied.contains(&m.version)).map(|m| m.version).collect();
    let Some(&to) = pending.iter().max() else { return Ok(None) };
    let from = applied.iter().max().copied().unwrap_or(0);
    let dir = data_dir.join("backups");
    tokio::fs::create_dir_all(&dir).await?;
    // 上次寫到一半（磁碟滿、斷電）留下的殘檔：不能還原，清掉
    for e in std::fs::read_dir(&dir)?.filter_map(|e| e.ok()) {
        if e.file_name().to_str().is_some_and(|n| n.starts_with("sorter-") && n.ends_with(".partial")) {
            let _ = std::fs::remove_file(e.path());
        }
    }
    // 檔名以時間開頭，照名稱排序就是新舊順序。先寫到暫存檔名，寫完才改成正式檔名：
    // 寫到一半失敗的檔案不可被當成可還原的備份，也不可擠掉真正完整的舊備份
    let name = format!("sorter-{}-v{from}-before-v{to}.sqlite", chrono::Local::now().format("%Y%m%d%H%M%S%3f"));
    let path = dir.join(&name);
    let partial = dir.join(format!("{name}.partial"));
    let started = std::time::Instant::now();
    if let Err(e) = sqlx::query("VACUUM INTO ?").bind(partial.to_string_lossy().into_owned()).execute(pool).await {
        let _ = std::fs::remove_file(&partial);
        return Err(e.into());
    }
    std::fs::rename(&partial, &path)?;
    tracing::warn!(path = %path.display(), ms = started.elapsed().as_millis() as u64, "套用資料表變更前已備份資料庫（v{from} → v{to}）；要降版請把這份放回去");
    let mut olds: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("sorter-") && n.ends_with(".sqlite")))
        .collect();
    olds.sort();
    let excess = olds.len().saturating_sub(BACKUPS_KEPT);
    for old in olds.into_iter().take(excess) {
        if let Err(e) = std::fs::remove_file(&old) {
            tracing::warn!(path = %old.display(), "舊的升級前備份刪不掉: {e}");
        }
    }
    Ok(Some(path))
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

    fn backups(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir.join("backups")).map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        v.sort();
        v
    }

    #[tokio::test]
    async fn 有新的資料表變更要套用時_先備份整個資料庫() {
        let dir = std::env::temp_dir().join(format!("bk-{}", ulid::Ulid::generate()));
        let pool = init(&dir).await.unwrap();
        sqlx::query("INSERT INTO parcels (ulid, barcode, status, started_at, started_ms, updated_ms) VALUES ('BK1', 'A1', 3, '2026-10-09 07:00:00.000', 1, 1)").execute(&pool).await.unwrap();
        let migrator = sqlx::migrate!("./migrations");
        assert_eq!(backup_before_migrate(&pool, &dir, &migrator).await.unwrap(), None, "全部已套用就不備份");
        // 模擬舊版資料庫：最後一個變更還沒套用
        let last = migrator.iter().map(|m| m.version).max().unwrap();
        sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ?").bind(last).execute(&pool).await.unwrap();
        let path = backup_before_migrate(&pool, &dir, &migrator).await.unwrap().expect("有待套用的變更要備份");
        let copy = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=ro", path.display())).await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM parcels").fetch_one(&copy).await.unwrap();
        assert_eq!(n, 1, "備份要有升級前的資料");
        // 輪替：只留最近 3 份
        for _ in 0..4 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            backup_before_migrate(&pool, &dir, &migrator).await.unwrap();
        }
        assert_eq!(backups(&dir).len(), 3, "{:?}", backups(&dir));
    }

    #[tokio::test]
    async fn 備份失敗時_照常升級_程式要起得來() {
        let dir = std::env::temp_dir().join(format!("bk-fail-{}", ulid::Ulid::generate()));
        let pool = init(&dir).await.unwrap();
        // 0011 只有 CREATE INDEX IF NOT EXISTS，可以安全地再套一次
        let pending = 11i64;
        sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ?").bind(pending).execute(&pool).await.unwrap();
        pool.close().await;
        // 備份目錄的位置被一個檔案占住：建不了目錄
        std::fs::write(dir.join("backups"), b"x").unwrap();
        let pool = init(&dir).await.expect("備份失敗也要能啟動");
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = ?").bind(pending).fetch_one(&pool).await.unwrap();
        assert_eq!(n, 1, "變更照常套用");
    }

    #[tokio::test]
    async fn 上次寫到一半的備份殘檔_不算進保留份數_下次備份時清掉() {
        let dir = std::env::temp_dir().join(format!("bk-partial-{}", ulid::Ulid::generate()));
        let pool = init(&dir).await.unwrap();
        let migrator = sqlx::migrate!("./migrations");
        let last = migrator.iter().map(|m| m.version).max().unwrap();
        sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ?").bind(last).execute(&pool).await.unwrap();
        // 磁碟滿時寫到一半的殘檔：檔名比任何正常備份都新
        std::fs::create_dir_all(dir.join("backups")).unwrap();
        std::fs::write(dir.join("backups/sorter-29991231235959999-v9-before-v13.sqlite.partial"), b"half").unwrap();
        for _ in 0..4 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            backup_before_migrate(&pool, &dir, &migrator).await.unwrap();
        }
        let names = backups(&dir);
        assert_eq!(names.iter().filter(|n| n.ends_with(".sqlite")).count(), 3, "{names:?}");
        assert!(!names.iter().any(|n| n.ends_with(".partial")), "殘檔要清掉：{names:?}");
    }

    #[tokio::test]
    async fn 全新的資料庫_不備份() {
        let dir = std::env::temp_dir().join(format!("bk-new-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rwc", dir.join("sorter.sqlite").display())).await.unwrap();
        assert_eq!(backup_before_migrate(&pool, &dir, &sqlx::migrate!("./migrations")).await.unwrap(), None);
        assert!(backups(&dir).is_empty());
    }
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
