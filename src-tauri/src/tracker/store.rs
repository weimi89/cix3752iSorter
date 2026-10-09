//! 狀態機的 DB 寫入端：獨立 task 依序寫，狀態機本身不等 I/O。
//!
//! 資料庫忙碌（等滿 busy_timeout 仍拿不到寫鎖）時每筆都退避重試：新增包裹失敗的話，之後這件的事件全部對不到。
//! 重試後仍失敗才記 event_log，不吞掉；狀態機的記憶體狀態不受影響。

use std::collections::HashMap;

use tokio::sync::mpsc;

use crate::db::{DbPool, local_ts, now_ms};
use crate::event_log::{self, Level};

use super::parcel::Parcel;

pub enum StoreOp {
    Insert(Parcel),
    Update(Parcel),
    Event { ulid: String, ts_ms: i64, source: &'static str, kind: String, raw: Option<String> },
    /// 終態已寫入，之後不會再有這件的更新；釋放 ulid→id 對應
    Forget(String),
    /// 終態件計入當日統計；帶當時的預設格口（異常口），歸類才知道件是不是落在異常口。
    /// 連同終態與「已計入」標記（`daily_counted`）同一個交易寫入：一件只計一次，沒計到的重啟收尾補計
    Daily(Parcel, String),
    /// 一次堵塞開始（統計用）
    Jam { ts_ms: i64, cart: u32, pos: i32, ulid: Option<String>, barcode: Option<String>, chute_code: Option<String> },
}

#[derive(Clone)]
pub struct Store {
    tx: mpsc::Sender<StoreOp>,
}

impl Store {
    /// 寫入工作登記在 `tasks`：所有 `Store` 放掉後它會把佇列寫完才結束，結束流程要等它
    pub fn spawn(db: DbPool, tasks: &tokio_util::task::TaskTracker) -> Self {
        let (tx, rx) = mpsc::channel(8192);
        crate::supervise::spawn_tracked("包裹寫入", tasks, run(db, rx));
        Self { tx }
    }

    pub fn send(&self, op: StoreOp) {
        if let Err(e) = self.tx.try_send(op) {
            tracing::error!("store 佇列滿，寫入丟失: {e}");
        }
    }

    pub fn insert(&self, p: &Parcel) {
        self.send(StoreOp::Insert(p.clone()));
    }

    pub fn update(&self, p: &Parcel) {
        self.send(StoreOp::Update(p.clone()));
    }

    pub fn event(&self, p: &Parcel, ts_ms: i64, source: &'static str, kind: impl Into<String>, raw: Option<String>) {
        self.send(StoreOp::Event { ulid: p.ulid.clone(), ts_ms, source, kind: kind.into(), raw });
    }

    pub fn forget(&self, p: &Parcel) {
        self.send(StoreOp::Forget(p.ulid.clone()));
    }
}

/// 補計的件：條碼、格口代號、格口來源、原因代碼、上線時間、狀態
type UncountedRow = (String, Option<String>, String, Option<String>, i64, i32);

/// 啟動時的收尾，同一個交易裡做兩件事：
/// - 上次沒收尾的在途件標成「失去追蹤」：重啟後分揀機已被重置，實體狀態無從得知
/// - 所有還沒計入當日統計的件補計一次（在途件、以及已結束但在計入前就關閉的件），照各自的終態歸類
///
/// 交易第一句必須是寫入：先 SELECT 會拿到讀快照，啟動時別的連線（事件紀錄）一寫，
/// 再升級成寫入就是 SQLITE_BUSY_SNAPSHOT（517），busy_timeout 不會重試，程式直接起不來
pub async fn close_orphans(db: &DbPool, default_chute: &str) -> anyhow::Result<u64> {
    use super::parcel::Status;
    let now = now_ms();
    let mut tx = db.begin().await?;
    // 沒有結束時間的件一定還沒計入（計入與寫結束時間在同一個交易）；明確標成未計入，
    // 升級前就在途、欄位預設為 1 的件才不會漏掉
    let closed = sqlx::query(
        "UPDATE parcels SET status = 4, ended_ms = ?1, travel_ms = ?1 - started_ms, updated_ms = ?1, daily_counted = 0,
                chute_source = CASE WHEN chute_source = 'pending' THEN 'timeout' ELSE chute_source END
          WHERE ended_ms IS NULL",
    )
    .bind(now)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let uncounted: Vec<UncountedRow> = sqlx::query_as(
        "UPDATE parcels SET daily_counted = 1 WHERE daily_counted = 0
          RETURNING barcode, chute_code, chute_source, chute_reason, started_ms, status",
    )
    .fetch_all(&mut *tx)
    .await?;
    for (barcode, chute_code, source, reason, started_ms, status) in &uncounted {
        let d = daily_delta(*started_ms, *status == Status::Done.code(), Some(barcode), chute_code.as_deref(), Some(source), reason.as_deref(), default_chute);
        add_daily(&mut *tx, &d).await?;
    }
    tx.commit().await?;
    Ok(closed)
}

async fn run(db: DbPool, mut rx: mpsc::Receiver<StoreOp>) {
    use crate::db::retry_busy;
    let mut ids: HashMap<String, i64> = HashMap::new();
    // 每件最後一次同步到照片的單號；沒變就不再寫照片表
    let mut synced: HashMap<String, Option<String>> = HashMap::new();
    while let Some(op) = rx.recv().await {
        match op {
            StoreOp::Insert(p) => match retry_busy("新增包裹", || insert(&db, &p)).await {
                Ok(id) => {
                    ids.insert(p.ulid.clone(), id);
                }
                Err(e) => event_log::log(&db, Level::Error, "tracker", "db_insert", format!("包裹 {} 寫入失敗: {e}", p.ulid)),
            },
            StoreOp::Update(p) => {
                let bc = image_barcode(&p).map(String::from);
                let sync = synced.get(&p.ulid) != Some(&bc);
                // 要同步照片時兩步同一個交易：分開提交的話中途結束會留下包裹有單號、照片沒有的永久不一致
                match retry_busy("更新包裹", || async {
                    let mut tx = db.begin().await?;
                    update(&mut tx, &p, sync).await?;
                    tx.commit().await
                })
                .await
                {
                    Ok(()) => {
                        synced.insert(p.ulid.clone(), bc);
                    }
                    Err(e) => event_log::log(&db, Level::Error, "tracker", "db_update", format!("包裹 {} 更新失敗: {e}", p.ulid)),
                }
            }
            StoreOp::Event { ulid, ts_ms, source, kind, raw } => {
                let Some(&id) = ids.get(&ulid) else {
                    tracing::warn!(%ulid, %kind, "事件對應不到包裹 id（插入尚未成功？）");
                    continue;
                };
                let r = retry_busy("包裹事件", || {
                    sqlx::query("INSERT INTO parcel_events (parcel_id, ts_ms, source, kind, raw) VALUES (?, ?, ?, ?, ?)")
                        .bind(id)
                        .bind(ts_ms)
                        .bind(source)
                        .bind(&kind)
                        .bind(&raw)
                        .execute(&db)
                })
                .await;
                if let Err(e) = r {
                    tracing::error!(%ulid, "parcel_events 寫入失敗: {e}");
                }
            }
            StoreOp::Forget(ulid) => {
                ids.remove(&ulid);
                synced.remove(&ulid);
            }
            StoreOp::Daily(p, default_chute) => {
                if let Err(e) = retry_busy("當日統計", || finish(&db, &p, &default_chute)).await {
                    event_log::log(&db, Level::Error, "tracker", "db_daily", format!("包裹 {} 計入當日統計失敗: {e}", p.ulid));
                }
            }
            StoreOp::Jam { ts_ms, cart, pos, ulid, barcode, chute_code } => {
                let parcel_id = ulid.as_ref().and_then(|u| ids.get(u).copied());
                let r = retry_busy("堵塞事件", || {
                    sqlx::query(
                        "INSERT INTO jam_events (ts_ms, created_at, cart, pos, module, parcel_id, barcode, chute_code) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    )
                    .bind(ts_ms)
                    .bind(local_ts(ts_ms))
                    .bind(cart as i64)
                    .bind(pos)
                    .bind(pos / 10 + 1)
                    .bind(parcel_id)
                    .bind(&barcode)
                    .bind(&chute_code)
                    .execute(&db)
                })
                .await;
                if let Err(e) = r {
                    tracing::error!("jam_events 寫入失敗: {e}");
                }
            }
        }
    }
}

/// 照片另存的單號：讀碼失敗或還沒綁碼時是空值
fn image_barcode(p: &Parcel) -> Option<&str> {
    p.barcode.as_deref().filter(|b| *b != crate::device::camera::NO_READ)
}

/// 終態、已計入標記與當日統計同一個交易：要嘛都寫進去、要嘛都沒有；已經計過的不再計
async fn finish(db: &DbPool, p: &Parcel, default_chute: &str) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    update(&mut tx, p, true).await?;
    let first = sqlx::query("UPDATE parcels SET daily_counted = 1 WHERE ulid = ? AND daily_counted = 0").bind(&p.ulid).execute(&mut *tx).await?.rows_affected() == 1;
    if first {
        daily(&mut *tx, p, default_chute).await?;
    }
    tx.commit().await
}

async fn insert(db: &DbPool, p: &Parcel) -> Result<i64, sqlx::Error> {
    let now = now_ms();
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO parcels (ulid, barcode, chute_source, status, belt_slot, started_at, started_ms, updated_ms, daily_counted)
         VALUES (?, ?, 'pending', ?, ?, ?, ?, ?, 0) RETURNING id",
    )
    .bind(&p.ulid)
    .bind(p.barcode_or_noread())
    .bind(p.status.code())
    .bind(p.slot as i64)
    .bind(local_ts(p.p_ms))
    .bind(p.p_ms)
    .bind(now)
    .fetch_one(db)
    .await?;
    Ok(row.0)
}

/// `sync_image_barcode`：條碼跟上次同步時不同才傳 true（條碼只在綁碼時會變，每次更新都同步是白寫）
async fn update(conn: &mut sqlx::SqliteConnection, p: &Parcel, sync_image_barcode: bool) -> Result<(), sqlx::Error> {
    let now = now_ms();
    let (code, cid, source, response_id, reason) = match &p.chute {
        Some(c) => (Some(c.code.clone()), Some(c.cid.0 as i64), serde_json::to_value(c.source).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "pending".into()), c.response_id, c.reason.clone()),
        None => (None, None, "pending".to_string(), None, None),
    };
    sqlx::query(
        "UPDATE parcels SET barcode = ?, chute_code = ?, chute_cid = ?, chute_source = ?, chute_reason = ?, status = ?, cart = ?,
            ir_length = ?, gap = ?, block_pos = ?, lost_pos = ?, response_id = ?, ended_ms = ?, travel_ms = ?, updated_ms = ?
         WHERE ulid = ?",
    )
    .bind(p.barcode_or_noread())
    .bind(code)
    .bind(cid)
    .bind(source)
    .bind(reason)
    .bind(p.status.code())
    .bind(p.cart.map(|c| c as i64))
    .bind(p.ir_length)
    .bind(p.gap)
    .bind(p.block_pos)
    .bind(p.lost_pos)
    .bind(response_id)
    .bind(p.ended_ms)
    .bind(p.ended_ms.map(|e| e - p.p_ms))
    .bind(now)
    .bind(&p.ulid)
    .execute(&mut *conn)
    .await?;
    // 照片另存的條碼跟著包裹走：包裹清掉後要靠它用單號找回照片。讀碼失敗沒有單號，存空值
    if sync_image_barcode {
        sqlx::query("UPDATE parcel_images SET barcode = ?1 WHERE parcel_id = (SELECT id FROM parcels WHERE ulid = ?2) AND barcode IS NOT ?1")
            .bind(image_barcode(p))
            .bind(&p.ulid)
            .execute(&mut *conn)
            .await?;
    }
    // 回報可能比包裹列早入列（那時查不到包裹、編號留空，之後隨時會被送出），寫入回報編號時補上，
    // 回報佇列頁才對得到條碼與格口
    if let Some(rid) = response_id {
        sqlx::query("UPDATE report_queue SET parcel_id = (SELECT id FROM parcels WHERE ulid = ?1) WHERE response_id = ?2 AND parcel_id IS NULL")
            .bind(&p.ulid)
            .bind(rid)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// 一件終態包裹對 `daily_stats` 的增量。即時收尾（狀態機）與重啟收尾（`close_orphans`）共用，兩邊才不會各算各的
struct DailyDelta {
    day: String,
    done: i64,
    noread: i64,
    defaulted: i64,
    abnormal: i64,
    middleware: i64,
    noread_landed: i64,
}

/// `chute_source` 用資料表裡的字串（`api`／`manual`／`timeout`…）；None = 還沒有格口決定
fn daily_delta(p_ms: i64, done: bool, barcode: Option<&str>, chute_code: Option<&str>, chute_source: Option<&str>, reason: Option<&str>, default_chute: &str) -> DailyDelta {
    use crate::db::abnormal_kind::{classify, AbnormalKind};
    let kind = classify(done, chute_code, barcode, reason, default_chute);
    DailyDelta {
        day: local_ts(p_ms).get(..10).unwrap_or_default().to_string(),
        done: done as i64,
        noread: barcode.is_none_or(|b| b == crate::device::camera::NO_READ) as i64,
        defaulted: chute_source.is_some_and(|s| !matches!(s, "api" | "manual")) as i64,
        abnormal: (!done) as i64,
        middleware: (kind == Some(AbnormalKind::Middleware)) as i64,
        noread_landed: (kind == Some(AbnormalKind::NoRead)) as i64,
    }
}

async fn daily<'e, E: sqlx::Executor<'e, Database = sqlx::Sqlite>>(ex: E, p: &Parcel, default_chute: &str) -> Result<(), sqlx::Error> {
    use super::parcel::Status;
    let source = p.chute.as_ref().and_then(|c| serde_json::to_value(c.source).ok()).and_then(|v| v.as_str().map(String::from));
    let d = daily_delta(
        p.p_ms,
        p.status == Status::Done,
        p.barcode.as_deref(),
        p.chute.as_ref().map(|c| c.code.as_str()),
        source.as_deref(),
        p.chute.as_ref().and_then(|c| c.reason.as_deref()),
        default_chute,
    );
    add_daily(ex, &d).await
}

async fn add_daily<'e, E: sqlx::Executor<'e, Database = sqlx::Sqlite>>(ex: E, d: &DailyDelta) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO daily_stats (day, total, done, noread, defaulted, abnormal, middleware, noread_landed) VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(day) DO UPDATE SET total = total + 1, done = done + ?2, noread = noread + ?3, defaulted = defaulted + ?4, abnormal = abnormal + ?5,
                                        middleware = middleware + ?6, noread_landed = noread_landed + ?7",
    )
    .bind(&d.day)
    .bind(d.done)
    .bind(d.noread)
    .bind(d.defaulted)
    .bind(d.abnormal)
    .bind(d.middleware)
    .bind(d.noread_landed)
    .execute(ex)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_db() -> DbPool {
        let dir = std::env::temp_dir().join(format!("store-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        crate::db::init(&dir).await.unwrap()
    }

    async fn seed_in_flight(db: &DbPool, barcode: &str, source: &str, chute: Option<&str>, started_ms: i64) {
        sqlx::query("INSERT INTO parcels (ulid, barcode, chute_code, chute_source, status, started_at, started_ms, updated_ms) VALUES (?, ?, ?, ?, 2, ?, ?, ?)")
            .bind(ulid::Ulid::generate().to_string()).bind(barcode).bind(chute).bind(source)
            .bind(local_ts(started_ms)).bind(started_ms).bind(started_ms)
            .execute(db).await.unwrap();
    }

    #[tokio::test]
    async fn 重啟收尾的在途件_也要計入當日統計_與包裹表件數一致() {
        let db = test_db().await;
        let t = now_ms() - 60_000;
        seed_in_flight(&db, "A0000000001", "api", Some("L1"), t).await;
        seed_in_flight(&db, "NoRead", "noread", Some("RS"), t + 1).await;
        seed_in_flight(&db, "B0000000002", "pending", None, t + 2).await;
        let closed = close_orphans(&db, "RS").await.unwrap();
        assert_eq!(closed, 3);
        let day = local_ts(t)[..10].to_string();
        let row: Option<(i64, i64, i64, i64, i64)> = sqlx::query_as("SELECT total, done, noread, defaulted, abnormal FROM daily_stats WHERE day = ?")
            .bind(&day).fetch_optional(&db).await.unwrap();
        assert_eq!(row, Some((3, 0, 1, 2, 3)), "3 件都失去追蹤；讀碼失敗 1；走預設口 2（noread、原本 pending 改成 timeout）");
        let parcels: i64 = sqlx::query_scalar("SELECT count(*) FROM parcels WHERE substr(started_at,1,10) = ?").bind(&day).fetch_one(&db).await.unwrap();
        assert_eq!(parcels, 3);
    }

    #[tokio::test]
    async fn 即時收尾的統計增量_各種情況與重構前公式一致() {
        use super::super::parcel::{ChuteDecision, ChuteSource, Status};
        use crate::protocol::Cid;
        let db = test_db().await;
        let t = now_ms();
        let mk = |status: Status, barcode: Option<&str>, chute: Option<(&str, ChuteSource, Option<&str>)>| {
            let mut p = Parcel::new(1, 1, t);
            p.status = status;
            p.barcode = barcode.map(String::from);
            p.chute = chute.map(|(code, source, reason)| ChuteDecision { code: code.into(), cid: Cid(1000323), source, response_id: None, decided_ms: t, reason: reason.map(String::from) });
            p
        };
        // (包裹, 期望 done, noread, defaulted, abnormal, middleware, noread_landed)——期望值照重構前的算式逐項算
        let cases = [
            (mk(Status::Done, Some("A1"), Some(("L1", ChuteSource::Api, None))), (1, 0, 0, 0, 0, 0)),
            (mk(Status::Done, Some("A2"), Some(("L1", ChuteSource::Manual, None))), (1, 0, 0, 0, 0, 0)),
            (mk(Status::Done, Some("A3"), Some(("RS", ChuteSource::Default, Some("STORE_CLOSED")))), (1, 0, 1, 0, 1, 0)),
            (mk(Status::Done, Some("NoRead"), Some(("RS", ChuteSource::NoRead, Some("NOREAD")))), (1, 1, 1, 0, 0, 1)),
            (mk(Status::Done, None, Some(("RS", ChuteSource::Timeout, Some("TIMEOUT")))), (1, 1, 1, 0, 0, 1)),
            (mk(Status::Done, Some("A4"), Some(("RS", ChuteSource::Default, Some("REENTRY")))), (1, 0, 1, 0, 0, 1)),
            (mk(Status::Lost, Some("A5"), Some(("L1", ChuteSource::Api, None))), (0, 0, 0, 1, 0, 0)),
            (mk(Status::Cancelled, Some("A6"), None), (0, 0, 0, 1, 0, 0)),
            (mk(Status::TriggerNg, None, Some(("L1", ChuteSource::Pending, None))), (0, 1, 1, 1, 0, 0)),
        ];
        for (i, (p, want)) in cases.iter().enumerate() {
            sqlx::query("DELETE FROM daily_stats").execute(&db).await.unwrap();
            daily(&db, p, "RS").await.unwrap();
            let got: (i64, i64, i64, i64, i64, i64) = sqlx::query_as("SELECT done, noread, defaulted, abnormal, middleware, noread_landed FROM daily_stats").fetch_one(&db).await.unwrap();
            assert_eq!(got, *want, "第 {i} 種情況");
        }
    }


    /// 已收尾、帶格口與回報編號的包裹
    fn finished_parcel(response_id: Option<i64>) -> Parcel {
        use super::super::parcel::{ChuteDecision, ChuteSource, Status};
        use crate::protocol::Cid;
        let t = now_ms() - 10_000;
        let mut p = Parcel::new(1, 1, t);
        p.status = Status::Done;
        p.barcode = Some("A0000000009".into());
        p.chute = Some(ChuteDecision { code: "L1".into(), cid: Cid(1000323), source: ChuteSource::Api, response_id, decided_ms: t + 300, reason: None });
        p.ended_ms = Some(t + 5_000);
        p
    }

    /// 起一個寫入端，送完 `ops` 後等它寫完
    async fn run_ops(db: &DbPool, ops: Vec<StoreOp>) {
        let tasks = tokio_util::task::TaskTracker::new();
        let store = Store::spawn(db.clone(), &tasks);
        for op in ops {
            store.send(op);
        }
        drop(store);
        assert!(crate::supervise::drain(&tasks, std::time::Duration::from_secs(60)).await, "寫入端要在時限內寫完");
    }

    #[tokio::test]
    async fn 包裹列晚於回報入列寫進來_寫入回報編號時補上回報的包裹編號() {
        let db = test_db().await;
        let p = finished_parcel(Some(5_501));
        // 回報先入列：這時包裹列還沒寫進來，包裹編號只能先留空（之後隨時可能已被送出）
        crate::middleware::report_queue::ReportQueue::new(db.clone()).enqueue(&p.ulid, 5_501).await.unwrap();
        run_ops(&db, vec![StoreOp::Insert(p.clone()), StoreOp::Update(p.clone())]).await;
        let pid: i64 = sqlx::query_scalar("SELECT id FROM parcels WHERE ulid = ?").bind(&p.ulid).fetch_one(&db).await.unwrap();
        let got: Option<i64> = sqlx::query_scalar("SELECT parcel_id FROM report_queue WHERE response_id = 5501").fetch_one(&db).await.unwrap();
        assert_eq!(got, Some(pid));
    }

    #[tokio::test]
    async fn 收尾的終態寫入遺失而統計寫入成功時_重啟不可再計一次() {
        let db = test_db().await;
        let p = finished_parcel(None);
        // 終態那筆 Update 失敗（只記錯誤）、統計那筆成功
        run_ops(&db, vec![StoreOp::Insert(p.clone()), StoreOp::Daily(p.clone(), "RS".into())]).await;
        close_orphans(&db, "RS").await.unwrap();
        let day = local_ts(p.p_ms)[..10].to_string();
        let total: i64 = sqlx::query_scalar("SELECT total FROM daily_stats WHERE day = ?").bind(&day).fetch_one(&db).await.unwrap();
        assert_eq!(total, 1, "同一件只能計一次");
        let status: i32 = sqlx::query_scalar("SELECT status FROM parcels WHERE ulid = ?").bind(&p.ulid).fetch_one(&db).await.unwrap();
        assert_eq!(status, super::super::parcel::Status::Done.code(), "計入統計的件不可被重啟收尾改成失去追蹤");
    }

    #[tokio::test]
    async fn 終態已寫入_計入統計前就關閉_重啟要補計一次() {
        let db = test_db().await;
        let p = finished_parcel(None);
        // 狀態機結束包裹時先寫終態，寬限 300ms 後才送計入統計；這段時間內關閉的話統計那筆根本沒送出
        run_ops(&db, vec![StoreOp::Insert(p.clone()), StoreOp::Update(p.clone())]).await;
        close_orphans(&db, "RS").await.unwrap();
        let day = local_ts(p.p_ms)[..10].to_string();
        let row: Option<(i64, i64)> = sqlx::query_as("SELECT total, done FROM daily_stats WHERE day = ?").bind(&day).fetch_optional(&db).await.unwrap();
        assert_eq!(row, Some((1, 1)), "照它自己的終態補計（完成）");
        let status: i32 = sqlx::query_scalar("SELECT status FROM parcels WHERE ulid = ?").bind(&p.ulid).fetch_one(&db).await.unwrap();
        assert_eq!(status, super::super::parcel::Status::Done.code(), "已結束的件不改成失去追蹤");
        close_orphans(&db, "RS").await.unwrap();
        let total: i64 = sqlx::query_scalar("SELECT total FROM daily_stats WHERE day = ?").bind(&day).fetch_one(&db).await.unwrap();
        assert_eq!(total, 1, "再重啟一次不可再計");
    }

    #[tokio::test]
    async fn 包裹條碼在照片對上之後才改_照片存的條碼跟著改() {
        let db = test_db().await;
        let mut p = finished_parcel(None);
        p.barcode = Some("OLD0000001".into());
        run_ops(&db, vec![StoreOp::Insert(p.clone())]).await;
        let pid: i64 = sqlx::query_scalar("SELECT id FROM parcels WHERE ulid = ?").bind(&p.ulid).fetch_one(&db).await.unwrap();
        sqlx::query("INSERT INTO parcel_images (parcel_id, barcode, file_name, rel_path, size, orig_size, received_ms, received_at) VALUES (?, 'OLD0000001', 'x.jpg', 'x.jpg', 1, 1, 1, '2026-10-09 07:30:00.000')")
            .bind(pid).execute(&db).await.unwrap();
        p.barcode = Some("NEW0000001".into());
        run_ops(&db, vec![StoreOp::Update(p.clone())]).await;
        let bc: Option<String> = sqlx::query_scalar("SELECT barcode FROM parcel_images").fetch_one(&db).await.unwrap();
        assert_eq!(bc.as_deref(), Some("NEW0000001"));
    }

    #[tokio::test]
    async fn 包裹條碼變成讀碼失敗_照片存的單號清成空值() {
        let db = test_db().await;
        let mut p = finished_parcel(None);
        p.barcode = Some("OLD0000002".into());
        run_ops(&db, vec![StoreOp::Insert(p.clone())]).await;
        let pid: i64 = sqlx::query_scalar("SELECT id FROM parcels WHERE ulid = ?").bind(&p.ulid).fetch_one(&db).await.unwrap();
        sqlx::query("INSERT INTO parcel_images (parcel_id, barcode, file_name, rel_path, size, orig_size, received_ms, received_at) VALUES (?, 'OLD0000002', 'x.jpg', 'x.jpg', 1, 1, 1, '2026-10-09 07:30:00.000')")
            .bind(pid).execute(&db).await.unwrap();
        p.barcode = None;
        run_ops(&db, vec![StoreOp::Update(p.clone())]).await;
        let bc: Option<String> = sqlx::query_scalar("SELECT barcode FROM parcel_images").fetch_one(&db).await.unwrap();
        assert_eq!(bc, None);
    }

    #[tokio::test]
    async fn 包裹條碼與照片條碼同一個交易_照片那步失敗時包裹也不改() {
        // 兩步分開提交的話，中途結束會留下包裹有單號、照片沒有的永久不一致（之後不會再同步）
        let db = test_db().await;
        let mut p = finished_parcel(None);
        p.barcode = Some("TXN0000001".into());
        run_ops(&db, vec![StoreOp::Insert(p.clone())]).await;
        let pid: i64 = sqlx::query_scalar("SELECT id FROM parcels WHERE ulid = ?").bind(&p.ulid).fetch_one(&db).await.unwrap();
        sqlx::query("INSERT INTO parcel_images (parcel_id, barcode, file_name, rel_path, size, orig_size, received_ms, received_at) VALUES (?, 'TXN0000001', 'x.jpg', 'x.jpg', 1, 1, 1, '2026-10-09 07:30:00.000')")
            .bind(pid).execute(&db).await.unwrap();
        sqlx::query("CREATE TRIGGER fail_image_update BEFORE UPDATE ON parcel_images BEGIN SELECT RAISE(ABORT, '模擬照片同步失敗'); END").execute(&db).await.unwrap();
        p.barcode = Some("TXN0000002".into());
        run_ops(&db, vec![StoreOp::Update(p.clone())]).await;
        let bc: String = sqlx::query_scalar("SELECT barcode FROM parcels WHERE id = ?").bind(pid).fetch_one(&db).await.unwrap();
        assert_eq!(bc, "TXN0000001", "照片同步失敗時包裹條碼也不可改");
    }

    #[tokio::test]
    async fn 正常收尾後重啟_不可再計() {
        let db = test_db().await;
        let p = finished_parcel(None);
        run_ops(&db, vec![StoreOp::Insert(p.clone()), StoreOp::Update(p.clone()), StoreOp::Daily(p.clone(), "RS".into()), StoreOp::Daily(p.clone(), "RS".into())]).await;
        close_orphans(&db, "RS").await.unwrap();
        let day = local_ts(p.p_ms)[..10].to_string();
        let total: i64 = sqlx::query_scalar("SELECT total FROM daily_stats WHERE day = ?").bind(&day).fetch_one(&db).await.unwrap();
        assert_eq!(total, 1, "同一件送兩次計入統計、再重啟，都只算一次");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn 寫鎖被握住超過等待上限時_新增包裹與事件要重試_不可丟件() {
        let db = test_db().await;
        let mut holder = db.acquire().await.unwrap();
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *holder).await.unwrap();
        let release = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(6_000)).await;
            sqlx::query("COMMIT").execute(&mut *holder).await.unwrap();
        });
        let p = finished_parcel(None);
        let ev = StoreOp::Event { ulid: p.ulid.clone(), ts_ms: p.p_ms, source: "camera", kind: "bind".into(), raw: None };
        run_ops(&db, vec![StoreOp::Insert(p.clone()), ev]).await;
        release.await.unwrap();
        let n: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM parcels), (SELECT count(*) FROM parcel_events)").fetch_one(&db).await.unwrap();
        assert_eq!(n, (1, 1), "包裹與事件都要寫進去");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn 重啟收尾時別的連線同時在寫_不可因讀快照升級失敗而起不來() {
        let db = test_db().await;
        for round in 0..30 {
            let t = now_ms() - 60_000;
            for k in 0..20 {
                seed_in_flight(&db, &format!("C{round:03}{k:03}"), "api", Some("L1"), t + k).await;
            }
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let writer = {
                let (db, stop) = (db.clone(), stop.clone());
                tokio::spawn(async move {
                    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                        let _ = sqlx::query("INSERT INTO event_log (level, category, action, message, created_at) VALUES ('info','t','t','x','2026-01-01')").execute(&db).await;
                    }
                })
            };
            tokio::task::yield_now().await;
            let r = close_orphans(&db, "RS").await;
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            writer.await.unwrap();
            assert_eq!(r.map_err(|e| e.to_string()).unwrap(), 20, "第 {round} 輪");
        }
        let (total, parcels): (i64, i64) = sqlx::query_as("SELECT (SELECT sum(total) FROM daily_stats), (SELECT count(*) FROM parcels)").fetch_one(&db).await.unwrap();
        assert_eq!(total, parcels);
    }


    #[test]
    fn 程式結束前要等寫入佇列寫完_不可讓已送出的包裹在關閉時遺失() {
        let dir = std::env::temp_dir().join(format!("drain-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        const N: usize = 300;
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        rt.block_on(async {
            let db = crate::db::init(&dir).await.unwrap();
            let tasks = tokio_util::task::TaskTracker::new();
            let store = Store::spawn(db.clone(), &tasks);
            for i in 0..N {
                store.insert(&Parcel::new(i as u64, 1, crate::db::now_ms()));
            }
            // 狀態機結束＝寫入入口全部放掉；接著結束流程等寫入做完
            drop(store);
            assert!(crate::supervise::drain(&tasks, std::time::Duration::from_secs(20)).await, "應在時限內寫完");
        });
        // 主程式回傳時 runtime 被丟掉，還沒做完的工作一律中止
        drop(rt);
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let n: i64 = rt.block_on(async {
            let db = crate::db::init(&dir).await.unwrap();
            sqlx::query_scalar("SELECT COUNT(*) FROM parcels").fetch_one(&db).await.unwrap()
        });
        assert_eq!(n, N as i64, "結束前送出的 {N} 件應全部寫進資料庫");
    }

}
