//! 統計綜合頁的彙總：一趟把區間內各種口徑算齊，前端只打一支 API。
//!
//! 兩種資料來源要分清楚，口徑才不會對不上：
//! - 每日件數、KPI、歷史比對走 `daily_stats`（永久保留，包裹結束時累加）。
//! - 格口、來源、狀態、時段、通過時間、列印、回報走 `parcels`／`print_jobs`／`report_queue`，
//!   受 `retention_days` 限制——區間早於保留天數的部分只有每日件數，其餘分項會偏少，
//!   回應帶 `parcels_since` 讓前端標示。
//!
//! 日期一律本機時間：`parcels.started_at` 是本機時間字串，直接比字串走索引；
//! `print_jobs.created_ms`／`report_queue.created_ms` 是 epoch 毫秒，區間邊界先換成本機日的毫秒。

use chrono::{Duration, Local, NaiveDate, TimeZone};
use serde::Serialize;
use sqlx::Row;

use crate::db::DbPool;

#[derive(Serialize, Default, Clone, Copy)]
pub struct Bucket {
    pub total: i64,
    pub done: i64,
    pub noread: i64,
    pub defaulted: i64,
    pub abnormal: i64,
}

#[derive(Serialize)]
pub struct Kpi {
    pub today: Bucket,
    pub yesterday: Bucket,
    pub last7: Bucket,
    pub last30: Bucket,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct DailyRow {
    pub day: String,
    pub total: i64,
    pub done: i64,
    pub noread: i64,
    pub defaulted: i64,
    pub abnormal: i64,
}

#[derive(Serialize, Default, Clone)]
pub struct HourBucket {
    pub hour: i64,
    pub total: i64,
    pub done: i64,
    pub abnormal: i64,
    pub noread: i64,
}

#[derive(Serialize)]
pub struct CartCount {
    pub cart: i64,
    pub total: i64,
    pub abnormal: i64,
}

/// 讀碼失敗依包裹長度（光電量到的長度單位）分桶：短件讀不到的比率高得多，現場調整讀碼站後可對照
#[derive(Serialize, Debug, PartialEq)]
pub struct LengthBucket {
    pub bucket: &'static str,
    pub total: i64,
    pub noread: i64,
}

#[derive(Serialize)]
pub struct HeatCell {
    /// 0 = 週日 … 6 = 週六（SQLite `%w`）
    pub weekday: i64,
    pub hour: i64,
    pub count: i64,
}

#[derive(Serialize)]
pub struct ChuteCount {
    pub code: String,
    pub label: String,
    pub total: i64,
    pub done: i64,
    pub abnormal: i64,
}

#[derive(Serialize)]
pub struct KeyCount {
    pub key: String,
    pub count: i64,
}

#[derive(Serialize)]
pub struct StatusCount {
    pub status: i64,
    pub count: i64,
}

#[derive(Serialize, Default)]
pub struct Travel {
    pub samples: i64,
    pub avg_ms: i64,
    pub p50_ms: i64,
    pub p90_ms: i64,
    pub p99_ms: i64,
    pub max_ms: i64,
}

#[derive(Serialize, Default)]
pub struct PrintStats {
    pub total: i64,
    pub done: i64,
    pub failed: i64,
    pub pending: i64,
    /// 印成功但試過不只一次
    pub retried: i64,
    pub by_printer: Vec<PrinterCount>,
}

#[derive(Serialize)]
pub struct PrinterCount {
    pub printer_port: String,
    /// 目前對照到這個埠位的格口（同一埠位可能對到多格）
    pub chutes: Vec<String>,
    pub total: i64,
    pub done: i64,
    pub failed: i64,
}

#[derive(Serialize, Default)]
pub struct ReportStats {
    pub total: i64,
    pub success: i64,
    pub failed: i64,
    pub pending: i64,
    /// 送成功但重試過
    pub retried: i64,
}

#[derive(Serialize)]
pub struct Compare {
    /// week = 近 7 天 vs 前 7 天；month = 近 30 天 vs 前 30 天
    pub label: &'static str,
    pub current: i64,
    pub previous: i64,
    /// 前期為 0 時無法比，回 None
    pub delta_ratio: Option<f64>,
}

#[derive(Serialize)]
pub struct ReasonCount {
    /// 原因代碼（見 migration 0003）；舊資料沒有代碼時由來源推回 NOREAD／TIMEOUT／OTHER
    pub key: String,
    pub count: i64,
    /// 其中走預設口的件數（其餘是帶錯誤提示面單、照分到正常格口的）
    pub defaulted: i64,
}

#[derive(Serialize, Default)]
pub struct JamStats {
    pub total: i64,
    /// 每千件的卡件次數
    pub per_thousand: f64,
    pub by_module: Vec<KeyCount>,
    pub by_hour: Vec<i64>,
}

#[derive(Serialize)]
pub struct StageStat {
    /// bind／api／accept／head／travel
    pub key: &'static str,
    pub samples: i64,
    pub p50_ms: i64,
    pub p90_ms: i64,
    pub p99_ms: i64,
    pub max_ms: i64,
    /// 超過預算的件數；沒有預算的段回 None
    pub budget_ms: Option<i64>,
    pub over_budget: i64,
}

#[derive(Serialize)]
pub struct ChuteTravel {
    pub code: String,
    pub samples: i64,
    pub p50_ms: i64,
    pub p90_ms: i64,
}

#[derive(Serialize)]
pub struct DeviceStat {
    pub device: &'static str,
    pub disconnects: i64,
    /// 最長一次斷線（毫秒）；斷到區間結束還沒連回來的算到現在
    pub longest_ms: i64,
    pub total_ms: i64,
    pub last_disconnect_at: Option<String>,
    pub by_hour: Vec<i64>,
}

#[derive(Serialize, Default)]
pub struct DuplicateStats {
    /// 上線 2 次以上的條碼數
    pub barcodes: i64,
    /// 多跑的次數（每個條碼的次數 - 1 加總）
    pub extra_runs: i64,
    pub twice: i64,
    pub thrice: i64,
    pub more: i64,
    pub top: Vec<DuplicateRow>,
}

#[derive(Serialize)]
pub struct DuplicateRow {
    pub barcode: String,
    pub count: i64,
    pub chutes: String,
}

#[derive(Serialize)]
pub struct Overview {
    pub from: String,
    pub to: String,
    pub retention_days: u32,
    /// `parcels` 表最早一筆的日期；區間起點早於它代表分項統計不完整。沒資料回 None
    pub parcels_since: Option<String>,
    pub kpi: Kpi,
    pub range: Bucket,
    pub daily: Vec<DailyRow>,
    pub hourly: Vec<HourBucket>,
    pub heatmap: Vec<HeatCell>,
    pub by_chute: Vec<ChuteCount>,
    pub by_source: Vec<KeyCount>,
    pub by_status: Vec<StatusCount>,
    pub travel: Travel,
    pub print: PrintStats,
    pub report: ReportStats,
    pub compare: Vec<Compare>,
    pub reasons: Vec<ReasonCount>,
    pub jams: JamStats,
    pub stages: Vec<StageStat>,
    pub travel_by_chute: Vec<ChuteTravel>,
    pub devices: Vec<DeviceStat>,
    pub duplicates: DuplicateStats,
    /// 各小車（分揀機載具）的件數與異常數，看有沒有某台特別會出事
    pub by_cart: Vec<CartCount>,
    pub noread_by_length: Vec<LengthBucket>,
    /// 這台有沒有印過任何面單。現場面單由中介機印時格口表雖然填了埠位、列印任務永遠是 0，
    /// 統計頁的列印卡就不顯示（看埠位有沒有設不準，所以看的是有沒有任務）
    pub printing_used: bool,
}

/// 各段耗時的預算（毫秒），與狀態機時間軸標紅的門檻一致
const BUDGET_API_MS: i64 = 1100;
const BUDGET_ACCEPT_MS: i64 = 300;
const BUDGET_HEAD_MS: i64 = 550;

/// 區間最長一年：再長的區間每日圖已經看不出東西，也免得一次掃太多列
pub const MAX_RANGE_DAYS: i64 = 366;

/// 解析 `YYYY-MM-DD`；空值當今天。回 Err 的訊息直接給前端看
pub fn parse_day(s: Option<&str>) -> Result<NaiveDate, String> {
    match s.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(Local::now().date_naive()),
        Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| format!("日期格式錯誤：{s}（應為 YYYY-MM-DD）")),
    }
}

/// 本機日 00:00 的 epoch 毫秒
fn day_start_ms(day: NaiveDate) -> i64 {
    let naive = day.and_hms_opt(0, 0, 0).expect("00:00:00 永遠合法");
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|t| t.timestamp_millis())
        .unwrap_or(0)
}

pub async fn overview(db: &DbPool, retention_days: u32, from: NaiveDate, to: NaiveDate) -> anyhow::Result<Overview> {
    if from > to {
        anyhow::bail!("起始日期不能晚於結束日期");
    }
    let span_days = (to - from).num_days() + 1;
    if span_days > MAX_RANGE_DAYS {
        anyhow::bail!("區間最長 {MAX_RANGE_DAYS} 天");
    }
    let today = Local::now().date_naive();
    let from_s = from.format("%Y-%m-%d").to_string();
    let to_s = to.format("%Y-%m-%d").to_string();
    // parcels.started_at 是 'YYYY-MM-DD HH:MM:SS.mmm'，用「隔天 00:00」當右開區間
    let ts_from = format!("{from_s} 00:00:00.000");
    let ts_to = format!("{} 00:00:00.000", (to + Duration::days(1)).format("%Y-%m-%d"));
    let ms_from = day_start_ms(from);
    let ms_to = day_start_ms(to + Duration::days(1));

    let parcels_since: Option<String> = sqlx::query_scalar("SELECT substr(MIN(started_at), 1, 10) FROM parcels")
        .fetch_one(db)
        .await?;

    let kpi = Kpi {
        today: daily_sum(db, today, today).await?,
        yesterday: daily_sum(db, today - Duration::days(1), today - Duration::days(1)).await?,
        last7: daily_sum(db, today - Duration::days(6), today).await?,
        last30: daily_sum(db, today - Duration::days(29), today).await?,
    };
    let range = daily_sum(db, from, to).await?;
    let daily = daily_rows(db, from, to).await?;

    let (hourly, heatmap) = hour_buckets(db, &ts_from, &ts_to).await?;
    let by_chute = chute_counts(db, &ts_from, &ts_to).await?;
    let by_source = source_counts(db, &ts_from, &ts_to).await?;
    let by_status = status_counts(db, &ts_from, &ts_to).await?;
    let travel = travel_stats(db, &ts_from, &ts_to).await?;
    let print = print_stats(db, ms_from, ms_to).await?;
    let report = report_stats(db, ms_from, ms_to).await?;

    let compare = vec![
        compare_period(db, today, 7, "week").await?,
        compare_period(db, today, 30, "month").await?,
    ];
    let reasons = reason_counts(db, &ts_from, &ts_to).await?;
    let jams = jam_stats(db, &ts_from, &ts_to).await?;
    let (stages, travel_by_chute) = stage_stats(db, &ts_from, &ts_to).await?;
    let devices = device_stats(db, &ts_from, &ts_to, ms_to.min(crate::db::now_ms())).await?;
    let duplicates = duplicate_stats(db, &ts_from, &ts_to).await?;
    let by_cart = cart_counts(db, &ts_from, &ts_to).await?;
    let noread_by_length = length_buckets(db, &ts_from, &ts_to).await?;
    let printing_used: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM print_jobs)").fetch_one(db).await?;

    Ok(Overview {
        from: from_s,
        to: to_s,
        retention_days,
        parcels_since,
        kpi,
        range,
        daily,
        hourly,
        heatmap,
        by_chute,
        by_source,
        by_status,
        travel,
        print,
        report,
        compare,
        reasons,
        jams,
        stages,
        travel_by_chute,
        devices,
        duplicates,
        by_cart,
        noread_by_length,
        printing_used: printing_used > 0,
    })
}

/// daily_stats 在 [from, to] 的加總
async fn daily_sum(db: &DbPool, from: NaiveDate, to: NaiveDate) -> anyhow::Result<Bucket> {
    let row = sqlx::query(
        "SELECT COALESCE(SUM(total),0) AS total, COALESCE(SUM(done),0) AS done, COALESCE(SUM(noread),0) AS noread,
                COALESCE(SUM(defaulted),0) AS defaulted, COALESCE(SUM(abnormal),0) AS abnormal
           FROM daily_stats WHERE day >= ? AND day <= ?",
    )
    .bind(from.format("%Y-%m-%d").to_string())
    .bind(to.format("%Y-%m-%d").to_string())
    .fetch_one(db)
    .await?;
    Ok(Bucket {
        total: row.try_get("total")?,
        done: row.try_get("done")?,
        noread: row.try_get("noread")?,
        defaulted: row.try_get("defaulted")?,
        abnormal: row.try_get("abnormal")?,
    })
}

/// [from, to] 每一天一列，沒件的日子補 0，前端畫圖不必補洞
async fn daily_rows(db: &DbPool, from: NaiveDate, to: NaiveDate) -> anyhow::Result<Vec<DailyRow>> {
    let rows: Vec<DailyRow> = sqlx::query_as(
        "SELECT day, total, done, noread, defaulted, abnormal FROM daily_stats WHERE day >= ? AND day <= ? ORDER BY day",
    )
    .bind(from.format("%Y-%m-%d").to_string())
    .bind(to.format("%Y-%m-%d").to_string())
    .fetch_all(db)
    .await?;
    let mut out = Vec::with_capacity(((to - from).num_days() + 1) as usize);
    let mut d = from;
    let mut it = rows.into_iter().peekable();
    while d <= to {
        let key = d.format("%Y-%m-%d").to_string();
        if it.peek().is_some_and(|r| r.day == key) {
            out.push(it.next().expect("peek 過"));
        } else {
            out.push(DailyRow { day: key, total: 0, done: 0, noread: 0, defaulted: 0, abnormal: 0 });
        }
        d += Duration::days(1);
    }
    Ok(out)
}

/// 區間內已終態的包裹依上線時刻分到 24 個小時桶，以及「星期 × 小時」熱力格。
/// 在途的不計：還沒結束的件既不算完成也不算異常，放進去會讓當下這一小時的異常數虛高。
async fn hour_buckets(db: &DbPool, ts_from: &str, ts_to: &str) -> anyhow::Result<(Vec<HourBucket>, Vec<HeatCell>)> {
    let rows = sqlx::query(
        "SELECT CAST(strftime('%w', substr(started_at, 1, 10)) AS INTEGER) AS wd,
                CAST(substr(started_at, 12, 2) AS INTEGER) AS h,
                COUNT(*) AS total, SUM(status = 3) AS done, SUM(barcode = 'NoRead') AS noread
           FROM parcels
          WHERE started_at >= ? AND started_at < ? AND ended_ms IS NOT NULL
          GROUP BY wd, h",
    )
    .bind(ts_from)
    .bind(ts_to)
    .fetch_all(db)
    .await?;
    let mut hourly: Vec<HourBucket> = (0..24).map(|h| HourBucket { hour: h, ..Default::default() }).collect();
    let mut heat = Vec::with_capacity(rows.len());
    for r in rows {
        let wd: i64 = r.try_get("wd")?;
        let h: i64 = r.try_get("h")?;
        let total: i64 = r.try_get("total")?;
        let done: i64 = r.try_get("done")?;
        let noread: i64 = r.try_get("noread")?;
        if let Some(b) = hourly.get_mut(h.clamp(0, 23) as usize) {
            b.total += total;
            b.done += done;
            b.abnormal += total - done;
            b.noread += noread;
        }
        heat.push(HeatCell { weekday: wd, hour: h, count: total });
    }
    Ok((hourly, heat))
}

/// 各格口件數，照格口表的排序；啟用中的格口即使 0 件也列出來，一眼看得出哪格沒在收
async fn chute_counts(db: &DbPool, ts_from: &str, ts_to: &str) -> anyhow::Result<Vec<ChuteCount>> {
    let rows = sqlx::query(
        "SELECT c.code, c.label, c.enabled,
                COUNT(p.id) AS total, COALESCE(SUM(p.status = 3), 0) AS done
           FROM chutes c
           LEFT JOIN parcels p ON p.chute_code = c.code AND p.started_at >= ? AND p.started_at < ? AND p.ended_ms IS NOT NULL
          GROUP BY c.code
          ORDER BY c.sort_order, c.code",
    )
    .bind(ts_from)
    .bind(ts_to)
    .fetch_all(db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let total: i64 = r.try_get("total")?;
        let done: i64 = r.try_get("done")?;
        let enabled: i64 = r.try_get("enabled")?;
        if total == 0 && enabled == 0 {
            continue;
        }
        out.push(ChuteCount { code: r.try_get("code")?, label: r.try_get("label")?, total, done, abnormal: total - done });
    }
    Ok(out)
}

/// 格口來源分布（api／default／noread／timeout／manual）
async fn source_counts(db: &DbPool, ts_from: &str, ts_to: &str) -> anyhow::Result<Vec<KeyCount>> {
    let rows = sqlx::query(
        "SELECT chute_source AS k, COUNT(*) AS n FROM parcels
          WHERE started_at >= ? AND started_at < ? AND ended_ms IS NOT NULL
          GROUP BY k ORDER BY n DESC",
    )
    .bind(ts_from)
    .bind(ts_to)
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|r| Ok(KeyCount { key: r.try_get("k")?, count: r.try_get("n")? }))
        .collect()
}

async fn status_counts(db: &DbPool, ts_from: &str, ts_to: &str) -> anyhow::Result<Vec<StatusCount>> {
    let rows = sqlx::query(
        "SELECT status, COUNT(*) AS n FROM parcels
          WHERE started_at >= ? AND started_at < ? AND ended_ms IS NOT NULL
          GROUP BY status ORDER BY status",
    )
    .bind(ts_from)
    .bind(ts_to)
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|r| Ok(StatusCount { status: r.try_get("status")?, count: r.try_get("n")? }))
        .collect()
}

/// 正常完成件從上線到落格口的耗時分位數。分位數用 OFFSET 取第 k 筆，不把整段 travel_ms 撈回來
async fn travel_stats(db: &DbPool, ts_from: &str, ts_to: &str) -> anyhow::Result<Travel> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n, COALESCE(AVG(travel_ms), 0) AS avg, COALESCE(MAX(travel_ms), 0) AS mx
           FROM parcels WHERE started_at >= ? AND started_at < ? AND status = 3 AND travel_ms IS NOT NULL",
    )
    .bind(ts_from)
    .bind(ts_to)
    .fetch_one(db)
    .await?;
    let samples: i64 = row.try_get("n")?;
    if samples == 0 {
        return Ok(Travel::default());
    }
    let avg: f64 = row.try_get("avg")?;
    let max_ms: i64 = row.try_get("mx")?;
    let pick = |q: f64| {
        let ts_from = ts_from.to_string();
        let ts_to = ts_to.to_string();
        // 第 ceil(n·q) 筆（1-based）＝ OFFSET ceil(n·q)-1；q=1 時就是最後一筆
        let offset = (((samples as f64) * q).ceil() as i64 - 1).clamp(0, samples - 1);
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT travel_ms FROM parcels
                  WHERE started_at >= ? AND started_at < ? AND status = 3 AND travel_ms IS NOT NULL
                  ORDER BY travel_ms LIMIT 1 OFFSET ?",
            )
            .bind(ts_from)
            .bind(ts_to)
            .bind(offset)
            .fetch_one(db)
            .await
        }
    };
    let p50 = pick(0.50).await?;
    let p90 = pick(0.90).await?;
    let p99 = pick(0.99).await?;
    Ok(Travel { samples, avg_ms: avg.round() as i64, p50_ms: p50, p90_ms: p90, p99_ms: p99, max_ms })
}

async fn print_stats(db: &DbPool, ms_from: i64, ms_to: i64) -> anyhow::Result<PrintStats> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS total,
                COALESCE(SUM(status = 'done'), 0) AS done,
                COALESCE(SUM(status = 'failed'), 0) AS failed,
                COALESCE(SUM(status IN ('pending','printing')), 0) AS pending,
                COALESCE(SUM(status = 'done' AND attempts > 1), 0) AS retried
           FROM print_jobs WHERE created_ms >= ? AND created_ms < ?",
    )
    .bind(ms_from)
    .bind(ms_to)
    .fetch_one(db)
    .await?;
    let mut out = PrintStats {
        total: row.try_get("total")?,
        done: row.try_get("done")?,
        failed: row.try_get("failed")?,
        pending: row.try_get("pending")?,
        retried: row.try_get("retried")?,
        by_printer: Vec::new(),
    };
    let rows = sqlx::query(
        "SELECT j.printer_port,
                COUNT(*) AS total,
                COALESCE(SUM(j.status = 'done'), 0) AS done,
                COALESCE(SUM(j.status = 'failed'), 0) AS failed,
                (SELECT GROUP_CONCAT(c.code, ',') FROM (SELECT code FROM chutes WHERE printer_port = j.printer_port ORDER BY sort_order) c) AS chutes
           FROM print_jobs j WHERE j.created_ms >= ? AND j.created_ms < ?
          GROUP BY j.printer_port ORDER BY j.printer_port",
    )
    .bind(ms_from)
    .bind(ms_to)
    .fetch_all(db)
    .await?;
    for r in rows {
        let chutes: Option<String> = r.try_get("chutes")?;
        out.by_printer.push(PrinterCount {
            printer_port: r.try_get("printer_port")?,
            chutes: chutes.map(|s| s.split(',').map(str::to_string).collect()).unwrap_or_default(),
            total: r.try_get("total")?,
            done: r.try_get("done")?,
            failed: r.try_get("failed")?,
        });
    }
    Ok(out)
}

async fn report_stats(db: &DbPool, ms_from: i64, ms_to: i64) -> anyhow::Result<ReportStats> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS total,
                COALESCE(SUM(status = 'success'), 0) AS success,
                COALESCE(SUM(status = 'failed'), 0) AS failed,
                COALESCE(SUM(status IN ('pending','sending')), 0) AS pending,
                COALESCE(SUM(status = 'success' AND retry_count > 0), 0) AS retried
           FROM report_queue WHERE created_ms >= ? AND created_ms < ?",
    )
    .bind(ms_from)
    .bind(ms_to)
    .fetch_one(db)
    .await?;
    Ok(ReportStats {
        total: row.try_get("total")?,
        success: row.try_get("success")?,
        failed: row.try_get("failed")?,
        pending: row.try_get("pending")?,
        retried: row.try_get("retried")?,
    })
}

/// 近 N 天（含今天）對前 N 天的總件數
async fn compare_period(db: &DbPool, today: NaiveDate, days: i64, label: &'static str) -> anyhow::Result<Compare> {
    let current = daily_sum(db, today - Duration::days(days - 1), today).await?.total;
    let previous = daily_sum(db, today - Duration::days(2 * days - 1), today - Duration::days(days)).await?.total;
    let delta_ratio = (previous > 0).then(|| (current - previous) as f64 / previous as f64);
    Ok(Compare { label, current, previous, delta_ratio })
}

/// 走預設口／帶錯誤面單的原因。舊資料沒有 chute_reason 時由來源推回：讀碼失敗／逾時／其他
async fn reason_counts(db: &DbPool, ts_from: &str, ts_to: &str) -> anyhow::Result<Vec<ReasonCount>> {
    let rows = sqlx::query(
        "SELECT COALESCE(chute_reason, CASE chute_source WHEN 'noread' THEN 'NOREAD' WHEN 'timeout' THEN 'TIMEOUT' WHEN 'default' THEN 'OTHER' END) AS k,
                COUNT(*) AS n, COALESCE(SUM(chute_source NOT IN ('api', 'manual')), 0) AS d
           FROM parcels
          WHERE started_at >= ? AND started_at < ? AND ended_ms IS NOT NULL
            AND (chute_reason IS NOT NULL OR chute_source IN ('noread', 'timeout', 'default'))
          GROUP BY k ORDER BY n DESC",
    )
    .bind(ts_from)
    .bind(ts_to)
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .filter_map(|r| {
            let key: Option<String> = r.try_get("k").ok()?;
            Some(Ok(ReasonCount { key: key?, count: r.try_get("n").ok()?, defaulted: r.try_get("d").ok()? }))
        })
        .collect()
}

/// 每千件的分母用明細表同一區間的件數，跟卡件事件同一份資料，超過保留天數的區間才不會分母大分子小
async fn jam_stats(db: &DbPool, ts_from: &str, ts_to: &str) -> anyhow::Result<JamStats> {
    let parcels_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM parcels WHERE started_at >= ? AND started_at < ?")
        .bind(ts_from)
        .bind(ts_to)
        .fetch_one(db)
        .await?;
    let rows = sqlx::query(
        "SELECT module, CAST(substr(created_at, 12, 2) AS INTEGER) AS h, COUNT(*) AS n
           FROM jam_events WHERE created_at >= ? AND created_at < ? GROUP BY module, h",
    )
    .bind(ts_from)
    .bind(ts_to)
    .fetch_all(db)
    .await?;
    let mut by_module: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    let mut by_hour = vec![0i64; 24];
    let mut total = 0;
    for r in rows {
        let module: i64 = r.try_get("module")?;
        let h: i64 = r.try_get("h")?;
        let n: i64 = r.try_get("n")?;
        *by_module.entry(module).or_default() += n;
        by_hour[h.clamp(0, 23) as usize] += n;
        total += n;
    }
    Ok(JamStats {
        total,
        per_thousand: if parcels_total > 0 { (total as f64 * 1000.0 / parcels_total as f64 * 10.0).round() / 10.0 } else { 0.0 },
        by_module: by_module.into_iter().map(|(m, n)| KeyCount { key: format!("M{m}"), count: n }).collect(),
        by_hour,
    })
}

fn percentile(sorted: &[i64], q: f64) -> i64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[(((sorted.len() as f64) * q).ceil() as usize).clamp(1, sorted.len()) - 1]
}

fn stage(key: &'static str, mut v: Vec<i64>, budget_ms: Option<i64>) -> StageStat {
    v.sort_unstable();
    let over = budget_ms.map(|b| v.iter().filter(|&&d| d > b).count() as i64).unwrap_or(0);
    StageStat {
        key,
        samples: v.len() as i64,
        p50_ms: percentile(&v, 0.5),
        p90_ms: percentile(&v, 0.9),
        p99_ms: percentile(&v, 0.99),
        max_ms: v.last().copied().unwrap_or(0),
        budget_ms,
        over_budget: over,
    }
}

/// 各段耗時：一條查詢把每件的四段時間差算出來，分位數在這裡算（區間內最多幾十萬列，記憶體夠）。
/// 段：相機綁碼（~P→條碼）、中介機回覆（~P→格口）、分揀機受理（Kn→~c）、頭部交接（~j→~g）、落格口（~P→~e，只算正常完成）
async fn stage_stats(db: &DbPool, ts_from: &str, ts_to: &str) -> anyhow::Result<(Vec<StageStat>, Vec<ChuteTravel>)> {
    let rows = sqlx::query(
        "SELECT p.chute_code, p.status, p.travel_ms,
                MAX(CASE WHEN pe.source = 'camera' AND pe.kind = 'bind' THEN pe.ts_ms END) - p.started_ms AS bind_ms,
                MAX(CASE WHEN pe.source = 'api' AND pe.kind = 'chute' THEN pe.ts_ms END) - p.started_ms AS api_ms,
                MAX(CASE WHEN pe.source = 'sorter' AND pe.kind = 'c' THEN pe.ts_ms END) - MAX(CASE WHEN pe.source = 'tracker' AND pe.kind = 'Kn' THEN pe.ts_ms END) AS accept_ms,
                MAX(CASE WHEN pe.source = 'sorter' AND pe.kind = 'g' THEN pe.ts_ms END) - MAX(CASE WHEN pe.source = 'sorter' AND pe.kind = 'j' THEN pe.ts_ms END) AS head_ms
           FROM parcels p JOIN parcel_events pe ON pe.parcel_id = p.id
          WHERE p.started_at >= ? AND p.started_at < ? AND p.ended_ms IS NOT NULL
          GROUP BY p.id",
    )
    .bind(ts_from)
    .bind(ts_to)
    .fetch_all(db)
    .await?;
    let (mut bind, mut api, mut accept, mut head, mut travel) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut by_chute: std::collections::BTreeMap<String, Vec<i64>> = std::collections::BTreeMap::new();
    for r in rows {
        // 相機比 ~P 早到的少數件會算出負數，那是綁碼窗口容許的，不算耗時
        let push = |v: &mut Vec<i64>, d: Option<i64>| {
            if let Some(d) = d.filter(|d| *d >= 0) {
                v.push(d);
            }
        };
        push(&mut bind, r.try_get("bind_ms").ok().flatten());
        push(&mut api, r.try_get("api_ms").ok().flatten());
        push(&mut accept, r.try_get("accept_ms").ok().flatten());
        push(&mut head, r.try_get("head_ms").ok().flatten());
        let status: i64 = r.try_get("status")?;
        if status == 3 {
            if let Some(t) = r.try_get::<Option<i64>, _>("travel_ms")? {
                travel.push(t);
                if let Some(code) = r.try_get::<Option<String>, _>("chute_code")? {
                    by_chute.entry(code).or_default().push(t);
                }
            }
        }
    }
    let stages = vec![
        stage("bind", bind, None),
        stage("api", api, Some(BUDGET_API_MS)),
        stage("accept", accept, Some(BUDGET_ACCEPT_MS)),
        stage("head", head, Some(BUDGET_HEAD_MS)),
        stage("travel", travel, None),
    ];
    let travel_by_chute = by_chute
        .into_iter()
        .map(|(code, mut v)| {
            v.sort_unstable();
            ChuteTravel { code, samples: v.len() as i64, p50_ms: percentile(&v, 0.5), p90_ms: percentile(&v, 0.9) }
        })
        .collect();
    Ok((stages, travel_by_chute))
}

/// 裝置斷線：從系統事件的連線／斷線配對算次數、最長與總斷線時間。
/// `now_ms` 是區間右界與現在取早者：斷到現在還沒回來的算到這一刻
async fn device_stats(db: &DbPool, ts_from: &str, ts_to: &str, now_ms: i64) -> anyhow::Result<Vec<DeviceStat>> {
    let rows = sqlx::query(
        "SELECT category, action, created_at FROM event_log
          WHERE created_at >= ? AND created_at < ? AND category IN ('belt', 'sorter', 'camera') AND action IN ('connected', 'disconnected')
          ORDER BY id",
    )
    .bind(ts_from)
    .bind(ts_to)
    .fetch_all(db)
    .await?;
    let parse_ms = |s: &str| {
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f")
            .ok()
            .and_then(|n| Local.from_local_datetime(&n).earliest())
            .map(|t| t.timestamp_millis())
    };
    let mut out: Vec<DeviceStat> = ["belt", "sorter", "camera"]
        .into_iter()
        .map(|d| DeviceStat { device: d, disconnects: 0, longest_ms: 0, total_ms: 0, last_disconnect_at: None, by_hour: vec![0; 24] })
        .collect();
    let mut open: [Option<i64>; 3] = [None; 3];
    for r in rows {
        let cat: String = r.try_get("category")?;
        let action: String = r.try_get("action")?;
        let at: String = r.try_get("created_at")?;
        let Some(i) = ["belt", "sorter", "camera"].iter().position(|d| *d == cat) else { continue };
        let Some(ms) = parse_ms(&at) else { continue };
        if action == "disconnected" {
            if open[i].is_none() {
                open[i] = Some(ms);
                out[i].disconnects += 1;
                if let Some(h) = at.get(11..13).and_then(|h| h.parse::<usize>().ok()) {
                    out[i].by_hour[h.min(23)] += 1;
                }
                out[i].last_disconnect_at = Some(at.clone());
            }
        } else if let Some(start) = open[i].take() {
            let d = (ms - start).max(0);
            out[i].total_ms += d;
            out[i].longest_ms = out[i].longest_ms.max(d);
        }
    }
    for (i, st) in out.iter_mut().enumerate() {
        if let Some(start) = open[i] {
            let d = (now_ms - start).max(0);
            st.total_ms += d;
            st.longest_ms = st.longest_ms.max(d);
        }
    }
    Ok(out)
}

/// 各小車的件數與異常數；沒對到小車的（還沒上車就結束）不算
async fn cart_counts(db: &DbPool, ts_from: &str, ts_to: &str) -> anyhow::Result<Vec<CartCount>> {
    let rows = sqlx::query(
        "SELECT cart, COUNT(*) AS n, COALESCE(SUM(status <> 3), 0) AS abn FROM parcels
          WHERE started_at >= ? AND started_at < ? AND ended_ms IS NOT NULL AND cart IS NOT NULL
          GROUP BY cart ORDER BY cart",
    )
    .bind(ts_from)
    .bind(ts_to)
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|r| Ok(CartCount { cart: r.try_get("cart")?, total: r.try_get("n")?, abnormal: r.try_get("abn")? }))
        .collect()
}

/// 讀碼失敗依包裹長度分桶；沒量到長度的（`~L` 沒來）不算
pub const LENGTH_BUCKETS: [&str; 4] = ["<15", "15-24", "25-34", ">=35"];

fn length_bucket(len: i64) -> &'static str {
    match len {
        l if l < 15 => LENGTH_BUCKETS[0],
        l if l < 25 => LENGTH_BUCKETS[1],
        l if l < 35 => LENGTH_BUCKETS[2],
        _ => LENGTH_BUCKETS[3],
    }
}

async fn length_buckets(db: &DbPool, ts_from: &str, ts_to: &str) -> anyhow::Result<Vec<LengthBucket>> {
    let rows = sqlx::query(
        "SELECT ir_length AS len, COUNT(*) AS n, COALESCE(SUM(barcode = 'NoRead'), 0) AS nr FROM parcels
          WHERE started_at >= ? AND started_at < ? AND ir_length IS NOT NULL AND ir_length > 0
          GROUP BY ir_length",
    )
    .bind(ts_from)
    .bind(ts_to)
    .fetch_all(db)
    .await?;
    let mut out: Vec<LengthBucket> = LENGTH_BUCKETS.iter().map(|b| LengthBucket { bucket: b, total: 0, noread: 0 }).collect();
    for r in rows {
        let len: i64 = r.try_get("len")?;
        let b = length_bucket(len);
        let slot = out.iter_mut().find(|o| o.bucket == b).expect("分桶名稱來自同一張表");
        slot.total += r.try_get::<i64, _>("n")?;
        slot.noread += r.try_get::<i64, _>("nr")?;
    }
    Ok(out)
}

/// 同一條碼在區間內上線 2 次以上（回流、重掃）
async fn duplicate_stats(db: &DbPool, ts_from: &str, ts_to: &str) -> anyhow::Result<DuplicateStats> {
    let rows = sqlx::query(
        "SELECT barcode, COUNT(*) AS n, GROUP_CONCAT(COALESCE(chute_code, '?'), ' → ') AS chutes
           FROM (SELECT barcode, chute_code FROM parcels WHERE started_at >= ? AND started_at < ? AND barcode <> 'NoRead' ORDER BY id)
          GROUP BY barcode HAVING n > 1 ORDER BY n DESC, barcode LIMIT 10000",
    )
    .bind(ts_from)
    .bind(ts_to)
    .fetch_all(db)
    .await?;
    let mut out = DuplicateStats::default();
    for r in rows {
        let n: i64 = r.try_get("n")?;
        out.barcodes += 1;
        out.extra_runs += n - 1;
        match n {
            2 => out.twice += 1,
            3 => out.thrice += 1,
            _ => out.more += 1,
        }
        if out.top.len() < 10 {
            out.top.push(DuplicateRow { barcode: r.try_get("barcode")?, count: n, chutes: r.try_get::<Option<String>, _>("chutes")?.unwrap_or_default() });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_db() -> DbPool {
        let dir = std::env::temp_dir().join(format!("stats-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        crate::db::init(&dir).await.unwrap()
    }

    /// 在 `day` 的 `hour` 點造一件包裹（已終態），順手把 daily_stats 累加，跟正式寫入路徑同口徑
    async fn seed_parcel(db: &DbPool, day: &str, hour: u32, chute: &str, source: &str, status: i64, travel_ms: i64) {
        let started_at = format!("{day} {hour:02}:15:00.000");
        let naive = chrono::NaiveDateTime::parse_from_str(&started_at, "%Y-%m-%d %H:%M:%S%.3f").unwrap();
        let ms = Local.from_local_datetime(&naive).earliest().unwrap().timestamp_millis();
        let barcode = if source == "noread" { "NoRead" } else { "SF001" };
        sqlx::query(
            "INSERT INTO parcels (ulid, barcode, chute_code, chute_source, status, started_at, started_ms, ended_ms, travel_ms, updated_ms)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(ulid::Ulid::generate().to_string())
        .bind(barcode)
        .bind(chute)
        .bind(source)
        .bind(status)
        .bind(&started_at)
        .bind(ms)
        .bind(ms + travel_ms)
        .bind(travel_ms)
        .bind(ms)
        .execute(db)
        .await
        .unwrap();
        let done = (status == 3) as i64;
        let noread = (source == "noread") as i64;
        let defaulted = !matches!(source, "api" | "manual") as i64;
        sqlx::query(
            "INSERT INTO daily_stats (day, total, done, noread, defaulted, abnormal) VALUES (?, 1, ?, ?, ?, ?)
             ON CONFLICT(day) DO UPDATE SET total = total + 1, done = done + ?, noread = noread + ?, defaulted = defaulted + ?, abnormal = abnormal + ?",
        )
        .bind(day)
        .bind(done).bind(noread).bind(defaulted).bind(1 - done)
        .bind(done).bind(noread).bind(defaulted).bind(1 - done)
        .execute(db)
        .await
        .unwrap();
    }

    fn day(offset: i64) -> String {
        (Local::now().date_naive() + Duration::days(offset)).format("%Y-%m-%d").to_string()
    }

    #[tokio::test]
    async fn 區間彙總_各口徑對得上造的資料() {
        let db = test_db().await;
        let today = day(0);
        let yesterday = day(-1);
        // 今天：L1 正常 ×2（10 點、14 點）、R2 走預設口逾時 1 件（14 點）、NoRead 走 RS 1 件（10 點，狀態 5 堵塞）
        seed_parcel(&db, &today, 10, "L1", "api", 3, 1000).await;
        seed_parcel(&db, &today, 14, "L1", "api", 3, 3000).await;
        seed_parcel(&db, &today, 14, "R2", "timeout", 3, 2000).await;
        seed_parcel(&db, &today, 10, "RS", "noread", 5, 500).await;
        // 昨天：1 件正常
        seed_parcel(&db, &yesterday, 9, "L2", "api", 3, 1500).await;
        // 列印與回報：今天 2 張印成功（其中 1 張重試）、1 張失敗；回報 2 成功 1 待送
        let now = crate::db::now_ms();
        for (status, attempts) in [("done", 1), ("done", 2), ("failed", 3)] {
            sqlx::query("INSERT INTO print_jobs (barcode, chute_code, printer_port, tspl_path, status, attempts, created_ms) VALUES ('x', 'L1', '1-8.1', '/dev/null', ?, ?, ?)")
                .bind(status).bind(attempts).bind(now).execute(&db).await.unwrap();
        }
        for (status, retry) in [("success", 0), ("success", 2), ("pending", 0)] {
            sqlx::query("INSERT INTO report_queue (response_id, payload, status, retry_count, created_ms) VALUES (1, '{}', ?, ?, ?)")
                .bind(status).bind(retry).bind(now).execute(&db).await.unwrap();
        }

        let from = Local::now().date_naive() - Duration::days(1);
        let to = Local::now().date_naive();
        let o = overview(&db, 15, from, to).await.unwrap();

        assert_eq!((o.range.total, o.range.done, o.range.noread, o.range.defaulted, o.range.abnormal), (5, 4, 1, 2, 1));
        assert_eq!((o.kpi.today.total, o.kpi.yesterday.total, o.kpi.last7.total), (4, 1, 5));
        assert_eq!(o.daily.len(), 2, "每日列要涵蓋區間每一天");
        assert_eq!((o.daily[0].day.as_str(), o.daily[0].total, o.daily[1].total), (yesterday.as_str(), 1, 4));

        let h10 = &o.hourly[10];
        let h14 = &o.hourly[14];
        assert_eq!((h10.total, h10.done, h10.abnormal, h10.noread), (2, 1, 1, 1));
        assert_eq!((h14.total, h14.done, h14.abnormal), (2, 2, 0));
        assert_eq!(o.heatmap.iter().map(|c| c.count).sum::<i64>(), 5);

        let l1 = o.by_chute.iter().find(|c| c.code == "L1").unwrap();
        assert_eq!((l1.total, l1.done, l1.abnormal), (2, 2, 0));
        assert!(o.by_chute.iter().any(|c| c.code == "L5" && c.total == 0), "啟用中但 0 件的格口也要列");
        assert_eq!(o.by_source.iter().find(|k| k.key == "api").map(|k| k.count), Some(3));
        assert_eq!(o.by_status.iter().find(|s| s.status == 5).map(|s| s.count), Some(1));

        // 正常完成 4 件：1000/1500/2000/3000 → p50 第 2 筆 1500、p90 第 4 筆 3000、平均 1875
        assert_eq!((o.travel.samples, o.travel.avg_ms, o.travel.p50_ms, o.travel.p90_ms, o.travel.p99_ms, o.travel.max_ms), (4, 1875, 1500, 3000, 3000, 3000));

        assert_eq!((o.print.total, o.print.done, o.print.failed, o.print.pending, o.print.retried), (3, 2, 1, 0, 1));
        assert_eq!(o.print.by_printer.len(), 1);
        assert_eq!(o.print.by_printer[0].chutes, vec!["L1".to_string()]);
        assert!(o.printing_used);
        assert!(o.by_cart.is_empty(), "造的資料沒填小車 → 不列");
        assert_eq!((o.report.total, o.report.success, o.report.failed, o.report.pending, o.report.retried), (3, 2, 0, 1, 1));

        let week = o.compare.iter().find(|c| c.label == "week").unwrap();
        assert_eq!((week.current, week.previous, week.delta_ratio), (5, 0, None), "前期 0 件不算比率");
        assert_eq!(o.parcels_since.as_deref(), Some(yesterday.as_str()));
    }

    #[tokio::test]
    async fn 原因_卡件_各段耗時_裝置_重複進線_各自對得上() {
        let db = test_db().await;
        let today = day(0);
        let to_ms = |hh: u32, mm: u32| {
            let n = chrono::NaiveDateTime::parse_from_str(&format!("{today} {hh:02}:{mm:02}:00.000"), "%Y-%m-%d %H:%M:%S%.3f").unwrap();
            Local.from_local_datetime(&n).earliest().unwrap().timestamp_millis()
        };
        // 同一條碼跑三次（L1、L1、R2）＋ 一件門市關轉走 RS ＋ 一件逾時後回覆太晚（LATE）＋ 一件讀碼失敗
        let mut ids = Vec::new();
        for (i, (barcode, chute, source, reason)) in [
            ("SF001", "L1", "api", None),
            ("SF001", "L1", "api", None),
            ("SF001", "R2", "api", None),
            ("SF002", "RS", "default", Some("STORE_CLOSED")),
            ("SF003", "RS", "timeout", Some("LATE")),
            ("NoRead", "RS", "noread", None), // 舊資料：沒有 reason，由來源推回 NOREAD
        ]
        .iter()
        .enumerate()
        {
            let ms = to_ms(10, i as u32);
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO parcels (ulid, barcode, chute_code, chute_source, chute_reason, status, started_at, started_ms, ended_ms, travel_ms, updated_ms)
                 VALUES (?, ?, ?, ?, ?, 3, ?, ?, ?, 2000, ?) RETURNING id",
            )
            .bind(format!("U{i}")).bind(barcode).bind(chute).bind(source).bind(reason)
            .bind(crate::db::local_ts(ms)).bind(ms).bind(ms + 2000).bind(ms)
            .fetch_one(&db).await.unwrap();
            ids.push((id, ms));
            // 每件：綁碼 +200、中介機回覆 +400（第 4 件 +1500 超預算）、Kn +1000、~c +1015、~j +1030、~g +1400
            let api_at = if i == 3 { 1500 } else { 400 };
            for (src, kind, dt) in [("belt", "P", 0), ("camera", "bind", 200), ("api", "chute", api_at), ("tracker", "Kn", 1000), ("sorter", "c", 1015), ("sorter", "j", 1030), ("sorter", "g", 1400)] {
                sqlx::query("INSERT INTO parcel_events (parcel_id, ts_ms, source, kind) VALUES (?, ?, ?, ?)")
                    .bind(id).bind(ms + dt).bind(src).bind(kind).execute(&db).await.unwrap();
            }
        }
        // 卡件：M4 兩次（10 點）、M1 一次（11 點）
        for (hh, pos, pid) in [(10, 35, Some(ids[0].0)), (10, 38, None), (11, 5, Some(ids[1].0))] {
            let ms = to_ms(hh, 30);
            sqlx::query("INSERT INTO jam_events (ts_ms, created_at, cart, pos, module, parcel_id) VALUES (?, ?, 1, ?, ?, ?)")
                .bind(ms).bind(crate::db::local_ts(ms)).bind(pos).bind(pos / 10 + 1).bind(pid).execute(&db).await.unwrap();
        }
        // 分揀機斷線兩次：10:00 斷 → 10:05 回、11:00 斷 → 11:02 回；皮帶 12:00 斷了沒回（算到現在）
        for (cat, action, hh, mm) in [("sorter", "disconnected", 10, 0), ("sorter", "connected", 10, 5), ("sorter", "disconnected", 11, 0), ("sorter", "connected", 11, 2), ("belt", "disconnected", 12, 0)] {
            sqlx::query("INSERT INTO event_log (level, category, action, message, created_at) VALUES ('warn', ?, ?, '', ?)")
                .bind(cat).bind(action).bind(crate::db::local_ts(to_ms(hh, mm))).execute(&db).await.unwrap();
        }
        let d = Local::now().date_naive();
        let o = overview(&db, 15, d, d).await.unwrap();

        let find = |k: &str| o.reasons.iter().find(|r| r.key == k).map(|r| (r.count, r.defaulted));
        assert_eq!(find("STORE_CLOSED"), Some((1, 1)));
        assert_eq!(find("LATE"), Some((1, 1)));
        assert_eq!(find("NOREAD"), Some((1, 1)), "舊資料沒代碼也要由來源推回");
        assert_eq!(o.reasons.len(), 3, "正常給格口的不算原因");

        assert_eq!((o.jams.total, o.jams.per_thousand), (3, 500.0), "6 件 3 次卡件 = 每千件 500 次");
        assert_eq!(o.jams.by_module.iter().map(|k| (k.key.as_str(), k.count)).collect::<Vec<_>>(), vec![("M1", 1), ("M4", 2)]);
        assert_eq!((o.jams.by_hour[10], o.jams.by_hour[11]), (2, 1));

        let st = |k: &str| o.stages.iter().find(|s| s.key == k).unwrap();
        assert_eq!((st("bind").samples, st("bind").p50_ms, st("bind").budget_ms), (6, 200, None));
        assert_eq!((st("api").samples, st("api").max_ms, st("api").over_budget), (6, 1500, 1), "一件 1500ms 超過 1100 預算");
        assert_eq!((st("accept").p50_ms, st("head").p50_ms, st("travel").p50_ms), (15, 370, 2000));
        let l1 = o.travel_by_chute.iter().find(|c| c.code == "L1").unwrap();
        assert_eq!((l1.samples, l1.p50_ms), (2, 2000));

        let dev = |k: &str| o.devices.iter().find(|d| d.device == k).unwrap();
        assert_eq!((dev("sorter").disconnects, dev("sorter").longest_ms, dev("sorter").total_ms), (2, 300_000, 420_000));
        assert_eq!((dev("sorter").by_hour[10], dev("sorter").by_hour[11]), (1, 1));
        assert_eq!(dev("belt").disconnects, 1);
        assert!(dev("belt").longest_ms > 0, "斷到現在還沒回來的要算到現在");
        assert_eq!(dev("camera").disconnects, 0);

        assert_eq!((o.duplicates.barcodes, o.duplicates.extra_runs, o.duplicates.twice, o.duplicates.thrice), (1, 2, 0, 1));
        sqlx::query("UPDATE parcels SET cart = CASE WHEN id % 2 = 0 THEN 7 ELSE 3 END, status = CASE WHEN id = ? THEN 4 ELSE status END").bind(ids[0].0).execute(&db).await.unwrap();
        let o2 = overview(&db, 15, d, d).await.unwrap();
        let cart = |c: i64| o2.by_cart.iter().find(|x| x.cart == c).map(|x| (x.total, x.abnormal));
        assert_eq!(o2.by_cart.len(), 2);
        assert_eq!(cart(3).unwrap().0 + cart(7).unwrap().0, 6);
        assert_eq!(cart(3).unwrap().1 + cart(7).unwrap().1, 1, "改成失去追蹤的那件算異常");
        assert_eq!((o.duplicates.top[0].barcode.as_str(), o.duplicates.top[0].count, o.duplicates.top[0].chutes.as_str()), ("SF001", 3, "L1 → L1 → R2"));
        assert!(!o.printing_used, "沒印過任何面單 → 列印卡不顯示");
    }

    #[tokio::test]
    async fn 沒資料時每個分項都是空或零_不報錯() {
        let db = test_db().await;
        let today = Local::now().date_naive();
        let o = overview(&db, 15, today, today).await.unwrap();
        assert_eq!(o.range.total, 0);
        assert_eq!(o.daily.len(), 1);
        assert_eq!(o.hourly.len(), 24);
        assert!(o.heatmap.is_empty());
        assert_eq!(o.travel.samples, 0);
        assert!(o.by_chute.iter().all(|c| c.total == 0));
        assert!(o.parcels_since.is_none());
        assert!(o.reasons.is_empty() && o.stages.iter().all(|s| s.samples == 0) && o.jams.total == 0 && o.duplicates.barcodes == 0);
        assert_eq!(o.devices.len(), 3);
    }

    #[tokio::test]
    async fn 區間檢查_起訖顛倒與超過一年都拒絕() {
        let db = test_db().await;
        let today = Local::now().date_naive();
        assert!(overview(&db, 15, today, today - Duration::days(1)).await.is_err());
        assert!(overview(&db, 15, today - Duration::days(400), today).await.is_err());
        assert_eq!(parse_day(Some("2026-13-01")).unwrap_err().contains("日期格式錯誤"), true);
        assert_eq!(parse_day(Some("")).unwrap(), today);
    }
    #[test]
    fn 包裹長度分桶邊界() {
        assert_eq!(length_bucket(0), "<15");
        assert_eq!(length_bucket(14), "<15");
        assert_eq!(length_bucket(15), "15-24");
        assert_eq!(length_bucket(24), "15-24");
        assert_eq!(length_bucket(25), "25-34");
        assert_eq!(length_bucket(35), ">=35");
        assert_eq!(length_bucket(300), ">=35");
    }

}
