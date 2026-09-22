//! 異常歸類：一件包裹只落一種，優先序「分揀機異常 > 仲介機回傳 > 讀碼失敗」。
//!
//! - 分揀機異常：件沒到該去的口（狀態 4–8：失去追蹤、堵塞、指令取消、觸發異常），責任在分揀機。
//! - 仲介機回傳：有讀到條碼，但仲介機說不能分（門市關轉、查無訂單、逾時未回…）或直接指到異常口，件正常落到異常口。
//! - 讀碼失敗：相機沒讀到碼、或讀到鄰件的碼被攔下（原因 REENTRY），件落到異常口。責任在讀碼站／投料。
//!
//! 統計、看板、異常口清單都從這裡取規則；各處自己判會出現同一件在不同頁算成不同類。
//! 「落到異常口」以實際落的格口（`chute_code = 預設格口`）判斷，不看格口來源——
//! 仲介機直接回異常口時來源是 api，看來源會把它算成正常件。

use crate::device::camera::NO_READ;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbnormalKind {
    Sorter,
    Middleware,
    NoRead,
}

impl AbnormalKind {
    /// API 與資料庫用的代碼
    pub fn as_str(self) -> &'static str {
        match self {
            AbnormalKind::Sorter => "sorter",
            AbnormalKind::Middleware => "middleware",
            AbnormalKind::NoRead => "noread",
        }
    }
}

/// 終態件的歸類；正常分揀完成回 `None`。
/// `done`＝狀態完成、`chute_code`＝實際落的格口、`default_chute`＝異常口（預設格口）。
pub fn classify(done: bool, chute_code: Option<&str>, barcode: Option<&str>, reason: Option<&str>, default_chute: &str) -> Option<AbnormalKind> {
    if !done {
        return Some(AbnormalKind::Sorter);
    }
    if chute_code != Some(default_chute) {
        return None;
    }
    if barcode.is_none_or(|b| b == NO_READ) || reason == Some("REENTRY") {
        Some(AbnormalKind::NoRead)
    } else {
        Some(AbnormalKind::Middleware)
    }
}

/// SQL：`parcels`（別名 `p`）這件屬於讀碼站那一類（沒讀到碼、或讀到鄰件的碼被攔）。
/// `chute_reason` 正常件是 NULL，要 COALESCE，否則 `NOT (...)` 會變成 NULL 整列消失
pub const SQL_NOREAD_KIND: &str = "(p.barcode = 'NoRead' OR COALESCE(p.chute_reason, '') = 'REENTRY')";

/// SQL：`parcels`（別名 `p`）已落異常口的列分兩種，配合 [`SQL_LANDED_DEFAULT`] 或 `p.chute_code = ?預設格口` 用
pub const SQL_LANDED_KIND: &str = "CASE WHEN (p.barcode = 'NoRead' OR COALESCE(p.chute_reason, '') = 'REENTRY') THEN 'noread' ELSE 'middleware' END";

/// SQL：`parcels`（別名 `p`）已終態的件的異常類別；正常完成回 NULL。配合 [`SQL_LANDED_DEFAULT`] 用（帶一個 `?` 異常口）
pub const SQL_ENDED_KIND: &str = "CASE WHEN p.status <> 3 THEN 'sorter'     WHEN (p.chute_source NOT IN ('api', 'manual') OR p.chute_code = ?) THEN (CASE WHEN (p.barcode = 'NoRead' OR COALESCE(p.chute_reason, '') = 'REENTRY') THEN 'noread' ELSE 'middleware' END)     ELSE NULL END";

/// 讀碼失敗的原因——只列系統分得出來的（照片看不出來的沒人分得出，不做人工標記）：
/// - `no_code`：讀碼器有拍、一個碼都沒讀到（面單朝下、看不到、反光皺褶…投料或讀碼站的問題）
/// - `neighbor`：讀到的是鄰件的碼（上一件面單還在畫面邊緣、或再進線被攔）→ 讀碼區域要排除進料區
/// - `bad_code`：讀到碼但都不是物流單號（包材條碼、內部序號）
/// - `no_frame`：讀碼器沒送任何結果（沒觸發、連線斷）
pub const NOREAD_CAUSES: &[&str] = &["no_code", "neighbor", "bad_code", "no_frame"];

/// SQL：`parcels`（別名 `p`）讀碼失敗那類的原因；不是讀碼失敗的列回 NULL。
/// 看綁碼事件的原始幀：沒事件＝沒送結果；幀裡沒有 `;`＝沒讀到碼；幀含前 10 秒內某件的條碼＝鄰件；其餘＝讀到的不是單號
pub const SQL_NOREAD_CAUSE: &str = "CASE     WHEN COALESCE(p.chute_reason, '') = 'REENTRY' THEN 'neighbor'     WHEN p.barcode <> 'NoRead' THEN NULL     WHEN NOT EXISTS (SELECT 1 FROM parcel_events e WHERE e.parcel_id = p.id AND e.source = 'camera' AND e.kind = 'bind') THEN 'no_frame'     WHEN (SELECT e.raw FROM parcel_events e WHERE e.parcel_id = p.id AND e.source = 'camera' AND e.kind = 'bind' ORDER BY e.id LIMIT 1) NOT LIKE '%;%' THEN 'no_code'     WHEN EXISTS (SELECT 1 FROM parcels q WHERE q.started_ms BETWEEN p.started_ms - 10000 AND p.started_ms AND q.id <> p.id AND q.barcode <> 'NoRead'                    AND instr((SELECT e.raw FROM parcel_events e WHERE e.parcel_id = p.id AND e.source = 'camera' AND e.kind = 'bind' ORDER BY e.id LIMIT 1), q.barcode) > 0) THEN 'neighbor'     ELSE 'bad_code' END";

/// SQL：`parcels`（別名 `p`）這件落在異常口。歷史資料不知道當時的異常口代碼，所以看兩個條件：
/// 來源不是 api／manual 的（讀碼失敗、逾時、仲介機沒給格口）當時一定走異常口；
/// 來源是 api 的只認「落在現在設定的異常口」。異常口改過代碼時，改名前仲介機直接回舊異常口的件會漏，其餘不受影響。
/// 第一個 `?` 綁現在的異常口代碼
pub const SQL_LANDED_DEFAULT: &str = "(p.chute_source NOT IN ('api', 'manual') OR p.chute_code = ?)";

/// 表 `daily_stats` 的 `middleware`／`noread_landed` 只有升到這版之後結束的件才會累加；
/// 之前的日子啟動時補一次：還在 `parcels` 的日子照規則算，已清掉的日子只能用舊欄位估
/// （`noread_landed` ≈ NoRead 件數、`middleware` ≈ 走預設口 − NoRead，兩者合計不超過完成數）。
/// 做過就記在 `app_setting`，之後啟動不再跑。日期比較用 `started_at >= day AND < day+1`（字串比較走索引），
/// 不用 `substr(started_at, 1, 10) = day`——那樣每一天都要掃整張 parcels。
pub async fn backfill_daily_stats(db: &super::DbPool, default_chute: &str) -> anyhow::Result<()> {
    const FLAG: &str = "daily_stats_kinds_backfilled";
    let done: Option<String> = sqlx::query_scalar("SELECT value FROM app_setting WHERE key = ?").bind(FLAG).fetch_optional(db).await?;
    if done.is_some() {
        return Ok(());
    }
    let mut tx = db.begin().await?;
    let exact = sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE daily_stats SET
            middleware = (SELECT COUNT(*) FROM parcels p WHERE p.started_at >= daily_stats.day AND p.started_at < date(daily_stats.day, '+1 day')
                           AND p.status = 3 AND NOT {1} AND {0}),
            noread_landed = (SELECT COUNT(*) FROM parcels p WHERE p.started_at >= daily_stats.day AND p.started_at < date(daily_stats.day, '+1 day')
                              AND p.status = 3 AND {1} AND {0})
          WHERE EXISTS (SELECT 1 FROM parcels p WHERE p.started_at >= daily_stats.day AND p.started_at < date(daily_stats.day, '+1 day'))",
        SQL_LANDED_DEFAULT, SQL_NOREAD_KIND
    )))
    .bind(default_chute)
    .bind(default_chute)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let estimated = sqlx::query(
        "UPDATE daily_stats SET
            noread_landed = MIN(noread, done),
            middleware = MIN(MAX(defaulted - noread, 0), done - MIN(noread, done))
          WHERE NOT EXISTS (SELECT 1 FROM parcels p WHERE p.started_at >= daily_stats.day AND p.started_at < date(daily_stats.day, '+1 day'))",
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    sqlx::query("INSERT INTO app_setting (key, value) VALUES (?, ?)")
        .bind(FLAG)
        .bind(format!("exact={exact},estimated={estimated}"))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(exact, estimated, "daily_stats 異常三分類回填完成");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 狀態不是完成一律算分揀機() {
        assert_eq!(classify(false, Some("RS"), None, None, "RS"), Some(AbnormalKind::Sorter));
        assert_eq!(classify(false, Some("L1"), Some("74Z1"), None, "RS"), Some(AbnormalKind::Sorter));
    }

    #[test]
    fn 落異常口依有沒有讀到碼分兩種() {
        assert_eq!(classify(true, Some("RS"), Some("74Z1"), Some("STORE_CLOSED"), "RS"), Some(AbnormalKind::Middleware));
        assert_eq!(classify(true, Some("RS"), Some(NO_READ), Some("NOREAD"), "RS"), Some(AbnormalKind::NoRead));
        assert_eq!(classify(true, Some("RS"), None, None, "RS"), Some(AbnormalKind::NoRead));
        assert_eq!(classify(true, Some("RS"), Some("74Z1"), Some("REENTRY"), "RS"), Some(AbnormalKind::NoRead), "讀到鄰件條碼被攔的算讀碼站那類");
    }

    async fn test_db() -> crate::db::DbPool {
        let dir = std::env::temp_dir().join(format!("kind-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        crate::db::init(&dir).await.unwrap()
    }

    async fn seed(db: &crate::db::DbPool, day: &str, barcode: &str, chute: &str, source: &str, status: i64) {
        sqlx::query(
            "INSERT INTO parcels (ulid, barcode, chute_code, chute_source, status, started_at, started_ms, ended_ms, updated_ms)
             VALUES (?, ?, ?, ?, ?, ?, 1, 2, 1)",
        )
        .bind(ulid::Ulid::generate().to_string())
        .bind(barcode)
        .bind(chute)
        .bind(source)
        .bind(status)
        .bind(format!("{day} 10:00:00.000"))
        .execute(db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn 回填_有明細的日子照規則算_沒明細的用舊欄位估_只跑一次() {
        let db = test_db().await;
        // 有明細的日子：正常 1、仲介機直接回異常口（來源 api）1、門市關轉 1、讀碼失敗落口 1、讀碼失敗被堵塞取走 1
        seed(&db, "2026-09-01", "A1", "L1", "api", 3).await;
        seed(&db, "2026-09-01", "A2", "RS", "api", 3).await;
        seed(&db, "2026-09-01", "A3", "RS", "default", 3).await;
        seed(&db, "2026-09-01", "NoRead", "RS", "noread", 3).await;
        seed(&db, "2026-09-01", "NoRead", "RS", "noread", 6).await;
        sqlx::query("INSERT INTO daily_stats (day, total, done, noread, defaulted, abnormal) VALUES ('2026-09-01', 5, 4, 2, 3, 1)").execute(&db).await.unwrap();
        // 明細已清的日子：只剩舊欄位。走預設口 40 件、NoRead 30 件（含 2 件狀態異常）→ 估 讀碼失敗 30、仲介機 10
        sqlx::query("INSERT INTO daily_stats (day, total, done, noread, defaulted, abnormal) VALUES ('2026-08-01', 1000, 995, 30, 40, 5)").execute(&db).await.unwrap();

        backfill_daily_stats(&db, "RS").await.unwrap();
        let (m, n): (i64, i64) = sqlx::query_as("SELECT middleware, noread_landed FROM daily_stats WHERE day = '2026-09-01'").fetch_one(&db).await.unwrap();
        assert_eq!((m, n), (2, 1), "api 直接回異常口也算仲介機回傳；被堵塞的 NoRead 不算落口");
        let (m, n): (i64, i64) = sqlx::query_as("SELECT middleware, noread_landed FROM daily_stats WHERE day = '2026-08-01'").fetch_one(&db).await.unwrap();
        assert_eq!((m, n), (10, 30));

        // 再跑一次不會重算：先把值改掉，看它有沒有被覆蓋
        sqlx::query("UPDATE daily_stats SET middleware = 99 WHERE day = '2026-09-01'").execute(&db).await.unwrap();
        backfill_daily_stats(&db, "RS").await.unwrap();
        let m: i64 = sqlx::query_scalar("SELECT middleware FROM daily_stats WHERE day = '2026-09-01'").fetch_one(&db).await.unwrap();
        assert_eq!(m, 99);
    }

    #[test]
    fn 正常格口完成不算異常_預設口改名也跟著() {
        assert_eq!(classify(true, Some("L1"), Some("74Z1"), None, "RS"), None);
        assert_eq!(classify(true, Some("RS"), Some("74Z1"), None, "NG"), None);
        assert_eq!(classify(true, Some("NG"), Some("74Z1"), None, "NG"), Some(AbnormalKind::Middleware));
    }
}
