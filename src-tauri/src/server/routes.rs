//! REST 端點。回應一律 JSON，錯誤回 `{ "error": "..." }`。
//!
//! 認證在 `auth.rs` 的中介層統一處理：內網免登入，外網要共用密碼；
//! 這裡只有「換門鎖」的兩件事（存取設定、密碼）另外要求來源必須在內網。

use std::net::SocketAddr;

use axum::{
    Json, Router,
    extract::{ConnectInfo, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;

use super::ServerState;
use crate::config::AppConfig;
use crate::event_log::{self, Level};
use crate::label::{tspl, usb};
use crate::tracker::ChuteRow;

pub struct ApiError(pub StatusCode, pub String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("資料庫錯誤: {e}"))
    }
}

pub type ApiResult<T> = Result<Json<T>, ApiError>;

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

fn unavailable(what: &str) -> ApiError {
    ApiError(StatusCode::SERVICE_UNAVAILABLE, format!("{what}尚未啟動"))
}


pub(super) fn api_router() -> Router<ServerState> {
    Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/parcels", get(parcels_list))
        .route("/parcels/export.xlsx", get(parcels_export))
        .route("/parcels/{id}", get(parcel_detail))
        .route("/parcel-images/{id}", get(parcel_image_meta))
        .route("/parcel-images/{id}/file", get(parcel_image_file))
        .route("/stats/daily", get(stats_daily))
        .route("/stats/hourly", get(stats_hourly))
        .route("/stats/overview", get(stats_overview))
        .route("/report/day", get(report_day))
        .route("/config", get(config_get).put(config_put))
        .route("/web-auth/password", get(web_auth_password_get).put(web_auth_password_put))
        .route("/chutes", get(chutes_get).put(chutes_put))
        .route("/abnormal/review", get(abnormal_review))
        .route("/belt/start", post(belt_start))
        .route("/belt/stop", post(belt_stop))
        .route("/sorter/reset", post(sorter_reset))
        .route("/sorter/command", post(sorter_command))
        .route("/ir/status", get(ir_status))
        .route("/ir/detail", post(ir_detail))
        .route("/ir/block", post(ir_block))
        .route("/devices/test", post(device_test))
        .route("/print-jobs", get(print_jobs))
        .route("/print-jobs/{id}/retry", post(print_job_retry))
        .route("/print-jobs/{id}/preview.png", get(print_job_preview))
        .route("/printers", get(printers))
        .route("/printers/{port}/test", post(printer_test))
        .route("/report-queue", get(report_queue))
        .route("/report-queue/{id}/retry", post(report_retry))
        .route("/logs", get(logs))
        .route("/logs/files", get(log_files))
        .route("/logs/files/{name}", get(log_file_download))
        .route("/lan-ips", get(lan_ips))
        .route("/client-errors", post(client_error))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

async fn status(State(state): State<ServerState>) -> ApiResult<serde_json::Value> {
    let rt = state.app.runtime.snapshot();
    let tracker = match state.app.tracker.get() {
        Some(h) => h.snapshot().await,
        None => None,
    };
    let (print_pending, print_failed): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(status IN ('pending','printing')),0), COALESCE(SUM(status='failed'),0) FROM print_jobs",
    )
    .fetch_one(&state.app.db)
    .await
    .unwrap_or((0, 0));
    let (report_pending, report_failed): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(status IN ('pending','sending')),0), COALESCE(SUM(status='failed'),0) FROM report_queue",
    )
    .fetch_one(&state.app.db)
    .await
    .unwrap_or((0, 0));
    // 近一小時讀碼失敗率：看板即時顯示，超過門檻標紅（2026-09-18 讀碼失敗率一晚從 1.9% 爬到 3.6% 沒人發現）
    let (noread_total, noread_count): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(barcode = 'NoRead'), 0) FROM parcels WHERE started_ms >= ?",
    )
    .bind(crate::db::now_ms() - 3_600_000)
    .fetch_one(&state.app.db)
    .await
    .unwrap_or((0, 0));
    Ok(Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": state.app.started_at.elapsed().as_secs(),
        "noread_1h": { "total": noread_total, "noread": noread_count },
        "devices": { "belt": rt.belt, "sorter": rt.sorter, "camera": rt.camera },
        "tracker": tracker,
        "chute_latency": state.app.resolver.get().map(|r| r.latency_stats()),
        "print": { "pending": print_pending, "failed": print_failed },
        "report": { "pending": report_pending, "failed": report_failed },
    })))
}

// ---------- 包裹 ----------

#[derive(Deserialize, Default)]
struct ParcelsQuery {
    #[serde(default)]
    start: Option<String>,
    #[serde(default)]
    end: Option<String>,
    #[serde(default)]
    barcode: Option<String>,
    #[serde(default)]
    chute: Option<String>,
    #[serde(default)]
    status: Option<i64>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default = "one")]
    page: i64,
    #[serde(default = "twenty")]
    limit: i64,
}

fn one() -> i64 {
    1
}
fn twenty() -> i64 {
    20
}

#[derive(serde::Serialize, sqlx::FromRow)]
struct ParcelRow {
    id: i64,
    ulid: String,
    barcode: String,
    chute_code: Option<String>,
    chute_cid: Option<i64>,
    chute_source: String,
    chute_reason: Option<String>,
    status: i64,
    belt_slot: Option<i64>,
    cart: Option<i64>,
    ir_length: Option<i64>,
    gap: Option<i64>,
    block_pos: Option<i64>,
    lost_pos: Option<i64>,
    response_id: Option<i64>,
    started_at: String,
    started_ms: i64,
    ended_ms: Option<i64>,
    travel_ms: Option<i64>,
    /// 這件的第一張讀碼站照片；NULL = 沒照片
    image_id: Option<i64>,
}

/// 多筆單號：逗號／分號／頓號／空白／換行都算分隔（貼 Excel 直欄也行）
fn split_tokens(s: &str) -> Vec<String> {
    s.split(|c: char| c == ',' || c == ';' || c == '、' || c.is_whitespace())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

/// 查詢條件一律參數化：`where_sql` 只由固定字串與 `?` 組成（使用者輸入全在 `binds`），
/// 所以下面用 `AssertSqlSafe` 包起來是安全的。
/// 條碼一筆 → 模糊比對；多筆 → 精確比對其中任一（現場貼一串單號查）。
fn parcels_filter(q: &ParcelsQuery) -> (String, Vec<String>) {
    let mut clauses = Vec::new();
    let mut binds = Vec::new();
    if let Some(s) = q.start.as_ref().filter(|s| !s.trim().is_empty()) {
        clauses.push("started_at >= ?".to_string());
        binds.push(s.trim().to_string());
    }
    if let Some(s) = q.end.as_ref().filter(|s| !s.trim().is_empty()) {
        clauses.push("started_at <= ?".to_string());
        binds.push(s.trim().to_string());
    }
    if let Some(s) = q.barcode.as_ref() {
        let tokens = split_tokens(s);
        match tokens.len() {
            0 => {}
            1 => {
                clauses.push("barcode LIKE ?".to_string());
                binds.push(format!("%{}%", tokens[0]));
            }
            n => {
                clauses.push(format!("barcode IN ({})", vec!["?"; n].join(",")));
                binds.extend(tokens);
            }
        }
    }
    if let Some(s) = q.chute.as_ref().filter(|s| !s.is_empty()) {
        clauses.push("chute_code = ?".to_string());
        binds.push(s.clone());
    }
    if let Some(s) = q.status {
        clauses.push("status = ?".to_string());
        binds.push(s.to_string());
    }
    if let Some(s) = q.source.as_ref().filter(|s| !s.is_empty()) {
        clauses.push("chute_source = ?".to_string());
        binds.push(s.clone());
    }
    let where_sql = if clauses.is_empty() { String::new() } else { format!(" WHERE {}", clauses.join(" AND ")) };
    (where_sql, binds)
}

const PARCEL_COLS: &str = "id, ulid, barcode, chute_code, chute_cid, chute_source, chute_reason, status, belt_slot, cart, ir_length, gap, block_pos, lost_pos, response_id, started_at, started_ms, ended_ms, travel_ms, (SELECT MIN(i.id) FROM parcel_images i WHERE i.parcel_id = parcels.id) AS image_id";

async fn parcels_list(State(state): State<ServerState>, Query(q): Query<ParcelsQuery>) -> ApiResult<serde_json::Value> {
    let (where_sql, binds) = parcels_filter(&q);
    let limit = q.limit.clamp(1, 500);
    let offset = (q.page.max(1) - 1) * limit;

    let mut count = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM parcels{where_sql}")));
    for b in &binds {
        count = count.bind(b);
    }
    let total = count.fetch_one(&state.app.db).await?;

    let mut list = sqlx::query_as::<_, ParcelRow>(sqlx::AssertSqlSafe(format!("SELECT {PARCEL_COLS} FROM parcels{where_sql} ORDER BY id DESC LIMIT ? OFFSET ?")));
    for b in &binds {
        list = list.bind(b);
    }
    let rows = list.bind(limit).bind(offset).fetch_all(&state.app.db).await?;
    Ok(Json(serde_json::json!({ "total": total, "list": rows })))
}

#[derive(serde::Serialize, sqlx::FromRow)]
struct EventRow {
    ts_ms: i64,
    source: String,
    kind: String,
    raw: Option<String>,
}

async fn parcel_detail(State(state): State<ServerState>, Path(id): Path<i64>) -> ApiResult<serde_json::Value> {
    let parcel = sqlx::query_as::<_, ParcelRow>(sqlx::AssertSqlSafe(format!("SELECT {PARCEL_COLS} FROM parcels WHERE id = ?")))
        .bind(id)
        .fetch_optional(&state.app.db)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "找不到這件包裹".into()))?;
    let events = sqlx::query_as::<_, EventRow>("SELECT ts_ms, source, kind, raw FROM parcel_events WHERE parcel_id = ? ORDER BY ts_ms, id")
        .bind(id)
        .fetch_all(&state.app.db)
        .await?;
    let prints: Vec<PrintJobRow> = sqlx::query_as("SELECT id, barcode, chute_code, printer_port, profile, status, attempts, last_error, not_before_ms, created_ms, finished_ms FROM print_jobs WHERE parcel_id = ? ORDER BY id")
        .bind(id)
        .fetch_all(&state.app.db)
        .await?;
    let images: Vec<ParcelImageRow> = sqlx::query_as("SELECT id, file_name, size, received_at, (orig_path IS NOT NULL) AS has_orig FROM parcel_images WHERE parcel_id = ? ORDER BY id")
        .bind(id)
        .fetch_all(&state.app.db)
        .await?;
    Ok(Json(serde_json::json!({ "parcel": parcel, "events": events, "print_jobs": prints, "images": images })))
}

#[derive(serde::Serialize, sqlx::FromRow)]
struct ParcelImageRow {
    id: i64,
    file_name: String,
    size: i64,
    received_at: String,
    has_orig: bool,
}

/// 放大檢視用：照片加上它對到的那件的條碼與時間（現場看照片比看資料多，標題要一眼認得出是哪件）
#[derive(serde::Serialize, sqlx::FromRow)]
struct ParcelImageMeta {
    id: i64,
    file_name: String,
    size: i64,
    received_at: String,
    has_orig: bool,
    parcel_id: Option<i64>,
    barcode: Option<String>,
    started_at: Option<String>,
    chute_code: Option<String>,
}

#[derive(Deserialize)]
struct ImageQuery {
    /// 1 = 拿保留的原圖（只有讀碼失敗件有）
    #[serde(default)]
    orig: u8,
}

/// 單張照片的資訊（列表只有 image_id，放大檢視要檔名與有沒有原圖）
async fn parcel_image_meta(State(state): State<ServerState>, Path(id): Path<i64>) -> ApiResult<ParcelImageMeta> {
    let row: Option<ParcelImageMeta> = sqlx::query_as(
        "SELECT i.id, i.file_name, i.size, i.received_at, (i.orig_path IS NOT NULL) AS has_orig,
                i.parcel_id, p.barcode, p.started_at, p.chute_code
           FROM parcel_images i LEFT JOIN parcels p ON p.id = i.parcel_id
          WHERE i.id = ?",
    )
        .bind(id)
        .fetch_optional(&state.app.db)
        .await?;
    Ok(Json(row.ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "找不到這張照片".into()))?))
}

/// 讀碼站照片本體；路徑只從資料表拿，不吃網址上的檔名
async fn parcel_image_file(State(state): State<ServerState>, Path(id): Path<i64>, Query(q): Query<ImageQuery>) -> Result<Response, ApiError> {
    let row: Option<(String, Option<String>)> = sqlx::query_as("SELECT rel_path, orig_path FROM parcel_images WHERE id = ?").bind(id).fetch_optional(&state.app.db).await?;
    let (rel, orig) = row.ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "找不到這張照片".into()))?;
    let rel = if q.orig == 1 { orig.ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "這件沒有保留原圖".into()))? } else { rel };
    let path = crate::device::camera_ftp::images_dir(&state.app.config.current().camera_ftp, &state.app.data_dir).join(&rel);
    let data = tokio::fs::read(&path).await.map_err(|_| ApiError(StatusCode::NOT_FOUND, "照片檔案已不存在".into()))?;
    let mime = mime_guess::from_path(&rel).first_or_octet_stream().to_string();
    Ok(([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "private, max-age=86400".to_string())], data).into_response())
}

async fn parcels_export(State(state): State<ServerState>, Query(q): Query<ParcelsQuery>) -> Result<Response, ApiError> {
    let (where_sql, binds) = parcels_filter(&q);
    let mut list = sqlx::query_as::<_, ParcelRow>(sqlx::AssertSqlSafe(format!("SELECT {PARCEL_COLS} FROM parcels{where_sql} ORDER BY id DESC LIMIT 50000")));
    for b in &binds {
        list = list.bind(b);
    }
    let rows = list.fetch_all(&state.app.db).await?;

    let bytes = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        use rust_xlsxwriter::Workbook;
        let mut wb = Workbook::new();
        let ws = wb.add_worksheet();
        let headers = ["編號", "條碼", "格口", "格口來源", "狀態", "開始時間", "結束時間", "耗時(ms)", "小車", "堵塞位置", "丟失位置", "回報ID"];
        for (i, h) in headers.iter().enumerate() {
            ws.write_string(0, i as u16, *h)?;
        }
        for (r, p) in rows.iter().enumerate() {
            let r = (r + 1) as u32;
            ws.write_number(r, 0, p.id as f64)?;
            ws.write_string(r, 1, &p.barcode)?;
            ws.write_string(r, 2, p.chute_code.as_deref().unwrap_or(""))?;
            ws.write_string(r, 3, &p.chute_source)?;
            ws.write_string(r, 4, status_label(p.status))?;
            ws.write_string(r, 5, &p.started_at)?;
            ws.write_string(r, 6, &p.ended_ms.map(crate::db::local_ts).unwrap_or_default())?;
            if let Some(t) = p.travel_ms {
                ws.write_number(r, 7, t as f64)?;
            }
            if let Some(c) = p.cart {
                ws.write_number(r, 8, c as f64)?;
            }
            if let Some(c) = p.block_pos {
                ws.write_number(r, 9, c as f64)?;
            }
            if let Some(c) = p.lost_pos {
                ws.write_number(r, 10, c as f64)?;
            }
            if let Some(c) = p.response_id {
                ws.write_number(r, 11, c as f64)?;
            }
        }
        Ok(wb.save_to_buffer()?)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    let name = format!("parcels_{}.xlsx", chrono::Local::now().format("%Y%m%d_%H%M%S"));
    Ok((
        [
            (header::CONTENT_TYPE, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
        ],
        bytes,
    )
        .into_response())
}

pub fn status_label(code: i64) -> &'static str {
    match code {
        1 => "初始化",
        2 => "收件",
        3 => "完成",
        4 => "失去追蹤",
        5 => "堵塞",
        6 => "堵塞後取走",
        7 => "指令取消",
        8 => "觸發異常",
        _ => "未知",
    }
}

#[derive(Deserialize)]
struct DaysQuery {
    #[serde(default = "thirty")]
    days: i64,
}
fn thirty() -> i64 {
    30
}

#[derive(serde::Serialize, sqlx::FromRow)]
struct DailyRow {
    day: String,
    total: i64,
    done: i64,
    noread: i64,
    defaulted: i64,
    abnormal: i64,
}

async fn stats_daily(State(state): State<ServerState>, Query(q): Query<DaysQuery>) -> ApiResult<Vec<DailyRow>> {
    let rows = sqlx::query_as::<_, DailyRow>("SELECT day, total, done, noread, defaulted, abnormal FROM daily_stats ORDER BY day DESC LIMIT ?")
        .bind(q.days.clamp(1, 365))
        .fetch_all(&state.app.db)
        .await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
struct HoursQuery {
    #[serde(default = "twelve")]
    hours: i64,
}
fn twelve() -> i64 {
    12
}

#[derive(serde::Serialize, Default, Clone)]
struct HourlyRow {
    /// 整點，本機時間 `YYYY-MM-DD HH:00`
    hour: String,
    total: i64,
    done: i64,
    /// 分揀機異常（狀態 4–8）；`middleware`／`noread_landed` 是落到異常口的兩類（規則在 `db::abnormal_kind`）
    abnormal: i64,
    middleware: i64,
    noread_landed: i64,
}

/// 最近 N 個整點（含當前這一小時）的完成／異常件數，依包裹上線時間分桶；
/// 沒有件的小時也回 0，前端直接畫圖不必補洞。只算已終態的包裹，在途的不計。
async fn stats_hourly(State(state): State<ServerState>, Query(q): Query<HoursQuery>) -> ApiResult<Vec<HourlyRow>> {
    use chrono::{Duration, Local, Timelike};
    let hours = q.hours.clamp(1, 168);
    let now = Local::now();
    let this_hour = now.with_minute(0).and_then(|t| t.with_second(0)).and_then(|t| t.with_nanosecond(0)).unwrap_or(now);
    let first = this_hour - Duration::hours(hours - 1);
    // started_at 是本機時間字串，直接比字串走 idx_parcels_started_at
    let since = first.format("%Y-%m-%d %H:%M:%S%.3f").to_string();
    let default_chute = state.app.config.current().general.default_chute;
    let landed = crate::db::abnormal_kind::SQL_LANDED_DEFAULT;
    let nr = crate::db::abnormal_kind::SQL_NOREAD_KIND;
    let rows: Vec<(String, i64, i64, i64, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT substr(p.started_at, 1, 13) AS h, COUNT(*), SUM(p.status = 3),
                COALESCE(SUM(p.status = 3 AND NOT {nr} AND {landed}), 0), COALESCE(SUM(p.status = 3 AND {nr} AND {landed}), 0)
         FROM parcels p WHERE p.started_at >= ? AND p.ended_ms IS NOT NULL GROUP BY h"
    )))
    .bind(&default_chute)
    .bind(&default_chute)
    .bind(&since)
    .fetch_all(&state.app.db)
    .await?;
    let mut out: Vec<HourlyRow> = (0..hours)
        .map(|i| HourlyRow { hour: (first + Duration::hours(i)).format("%Y-%m-%d %H:00").to_string(), ..Default::default() })
        .collect();
    for (h, total, done, middleware, noread_landed) in rows {
        if let Some(b) = out.iter_mut().find(|b| b.hour.starts_with(&h)) {
            b.total = total;
            b.done = done;
            b.abnormal = total - done;
            b.middleware = middleware;
            b.noread_landed = noread_landed;
        }
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
struct RangeQuery {
    from: Option<String>,
    to: Option<String>,
}

/// 統計綜合頁：區間內所有口徑一次算齊（見 `stats.rs`）。日期缺省為今天；格式或區間不合法回 400
async fn stats_overview(State(state): State<ServerState>, Query(q): Query<RangeQuery>) -> ApiResult<super::stats::Overview> {
    let from = super::stats::parse_day(q.from.as_deref()).map_err(bad)?;
    let to = super::stats::parse_day(q.to.as_deref()).map_err(bad)?;
    let general = state.app.config.current().general;
    let overview = super::stats::overview(&state.app.db, general.retention_days, &general.default_chute, from, to)
        .await
        .map_err(|e| bad(e.to_string()))?;
    Ok(Json(overview))
}

/// 班次報表：一天的統計＋依門檻挑出的問題點；`day` 空白＝最近有資料的那天（今天沒件就退到昨天）
async fn report_day(State(state): State<ServerState>, Query(q): Query<RangeQuery>) -> ApiResult<serde_json::Value> {
    let day = match q.from.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => super::stats::parse_day(Some(s)).map_err(bad)?,
        None => {
            let latest: Option<String> = sqlx::query_scalar("SELECT day FROM daily_stats WHERE total > 0 ORDER BY day DESC LIMIT 1").fetch_optional(&state.app.db).await?;
            super::stats::parse_day(latest.as_deref()).map_err(bad)?
        }
    };
    let general = state.app.config.current().general;
    let overview = super::stats::overview(&state.app.db, general.retention_days, &general.default_chute, day, day).await.map_err(|e| bad(e.to_string()))?;
    let findings = super::report::findings(&overview);
    // 前一天／後一天有沒有資料，前端做翻頁用
    let (prev, next): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT (SELECT MAX(day) FROM daily_stats WHERE total > 0 AND day < ?1), (SELECT MIN(day) FROM daily_stats WHERE total > 0 AND day > ?1)",
    )
    .bind(day.format("%Y-%m-%d").to_string())
    .fetch_one(&state.app.db)
    .await?;
    Ok(Json(serde_json::json!({ "day": overview.from, "prev_day": prev, "next_day": next, "findings": findings, "overview": overview })))
}

// ---------- 設定 ----------

async fn config_get(State(state): State<ServerState>) -> ApiResult<AppConfig> {
    Ok(Json(state.app.config.current()))
}

async fn config_put(
    State(state): State<ServerState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(cfg): Json<AppConfig>,
) -> ApiResult<serde_json::Value> {
    let external = super::auth::guard_web_access_change(&state, peer.ip(), &cfg.web_access).map_err(|m| ApiError(StatusCode::FORBIDDEN, m))?;
    for b in &cfg.emergency_buttons {
        if b.bit > 7 {
            return Err(bad(format!("按鈕「{}」的位元必須在 0–7", b.describe)));
        }
    }
    cfg.server.bind.parse::<std::net::SocketAddr>().map_err(|_| bad("網頁監聽位址格式錯誤"))?;
    for c in &cfg.web_access.lan_cidrs {
        c.parse::<ipnet::IpNet>().map_err(|_| bad(format!("內網網段「{c}」不是有效的 CIDR（例如 192.168.0.0/16）")))?;
    }
    // 安全參數的上下限不能只靠畫面：設定檔可以手改、API 可以直接打；超出範圍的值會讓鎖定形同虛設
    // 或讓 SQLite 的時間運算回 NULL
    let f = &cfg.camera_ftp;
    f.listen.parse::<std::net::SocketAddr>().map_err(|_| bad("讀碼站照片 FTP 監聽位址格式錯誤"))?;
    if f.enabled && (f.username.trim().is_empty() || f.password.is_empty()) {
        return Err(bad("讀碼站照片 FTP 帳號與密碼不可空白"));
    }
    if !(1..=100).contains(&f.jpeg_quality) {
        return Err(bad("證據圖 JPEG 品質必須在 1–100"));
    }
    if f.max_edge_px != 0 && !(320..=8000).contains(&f.max_edge_px) {
        return Err(bad("證據圖長邊必須是 0（不縮）或 320–8000 像素"));
    }
    if (f.passive_port_min == 0) != (f.passive_port_max == 0) || f.passive_port_min > f.passive_port_max {
        return Err(bad("被動模式埠範圍要兩個都填（起 ≤ 迄），或兩個都 0 交給系統"));
    }
    if !(500..=60_000).contains(&f.match_window_ms) {
        return Err(bad("照片對包裹的時間窗口必須在 500–60000 毫秒"));
    }
    if f.retention_days > 3650 {
        return Err(bad("照片保留天數最多 3650"));
    }
    let dir = f.images_dir.trim();
    if !dir.is_empty() {
        let p = std::path::Path::new(dir);
        if !p.is_absolute() {
            return Err(bad("照片存放目錄要填絕對路徑（例如 /mnt/photos），空白 = 資料目錄下的 images"));
        }
        // 換目錄不搬舊圖，但目錄至少要建得出來，否則相機一上傳就全數失敗
        std::fs::create_dir_all(p).map_err(|e| bad(format!("照片存放目錄建立失敗：{e}")))?;
    }
    if !(0..=60_000).contains(&cfg.general.reentry_hold_ms) {
        return Err(bad("同碼再進線攔截秒數必須在 0–60 秒（0 = 關閉）"));
    }
    if (cfg.camera.frame_width == 0) != (cfg.camera.frame_height == 0) || cfg.camera.frame_width > 20_000 || cfg.camera.frame_height > 20_000 {
        return Err(bad("相機畫面尺寸要兩邊都填（像素，最大 20000），或兩邊都 0 不用座標挑碼"));
    }
    let w = &cfg.web_access;
    if !(1..=720).contains(&w.session_hours) {
        return Err(bad("登入後可用時數必須在 1–720 小時"));
    }
    if !(1..=50).contains(&w.max_fail_attempts) {
        return Err(bad("密碼可錯次數必須在 1–50"));
    }
    if !(1..=1440).contains(&w.lock_minutes) {
        return Err(bad("鎖住分鐘數必須在 1–1440"));
    }
    // 外網來源：不管送來的 web_access 長怎樣，一律以拿到寫入鎖當下的現況為準——
    // 只比對不覆蓋的話，外網拿著舊快照就能把內網剛改好的設定蓋回去
    state
        .app
        .config
        .update_with(move |current| {
            let mut next = cfg;
            if external {
                next.web_access = current.web_access.clone();
            }
            next
        })
        .await?;
    event_log::log(&state.app.db, Level::Info, "server", "config", String::from("設定已更新"));
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ClientErrorBody {
    message: String,
    #[serde(default)]
    stack: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    page: String,
    #[serde(default)]
    runtime: String,
    #[serde(default)]
    ua: String,
}

/// 前端（桌面視窗或網頁版）把畫面上的 JavaScript 錯誤送回來記進事件記錄。
///
/// 正式版桌面沒有開發者工具，元件出錯只會默默不顯示（例如 2026-09-18 現場列印任務頁的
/// 日期與條碼欄位整個消失），現場回報時完全沒有線索。記成 `ui` 類別的警告，事件記錄頁就查得到
/// 錯誤訊息、堆疊、出事的頁面與環境。
///
/// 只收本機／內網／已登入來源（guard 已擋），欄位一律截斷、每分鐘最多 30 筆——前端若陷入錯誤迴圈，
/// 不能把事件記錄灌爆。
async fn client_error(State(state): State<ServerState>, Json(body): Json<ClientErrorBody>) -> ApiResult<serde_json::Value> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static BUDGET: Mutex<Option<(Instant, u32)>> = Mutex::new(None);
    const PER_MINUTE: u32 = 30;
    {
        let mut b = BUDGET.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let (start, n) = match *b {
            Some((start, n)) if now.duration_since(start) < Duration::from_secs(60) => (start, n),
            _ => (now, 0),
        };
        if n >= PER_MINUTE {
            return Ok(Json(serde_json::json!({ "ok": false, "reason": "rate_limited" })));
        }
        *b = Some((start, n + 1));
    }
    let cut = |s: &str, n: usize| -> String { s.chars().take(n).collect() };
    let mut message = format!(
        "[{}] {}：{}",
        cut(&body.runtime, 16),
        cut(&body.page, 120),
        cut(body.message.trim(), 500)
    );
    if !body.source.trim().is_empty() {
        message.push_str(&format!("（{}）", cut(body.source.trim(), 160)));
    }
    if !body.stack.trim().is_empty() {
        message.push('\n');
        message.push_str(&cut(body.stack.trim(), 2000));
    }
    if !body.ua.trim().is_empty() {
        message.push('\n');
        message.push_str(&cut(body.ua.trim(), 200));
    }
    event_log::log(&state.app.db, Level::Warn, "ui", "error", message);
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// 是否已設定網頁存取密碼（不回雜湊本身）
async fn web_auth_password_get(State(state): State<ServerState>) -> ApiResult<serde_json::Value> {
    let set = super::auth::stored_password_hash(&state.app.db).await?.is_some();
    Ok(Json(serde_json::json!({ "password_set": set })))
}

#[derive(Deserialize)]
struct SetPasswordBody {
    password: String,
}

/// 設定或清除網頁存取密碼；只准在工控機本機或現場網路內做（見 auth::guard_lan_only）
async fn web_auth_password_put(
    State(state): State<ServerState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<SetPasswordBody>,
) -> ApiResult<serde_json::Value> {
    super::auth::guard_lan_only(&state, peer.ip(), "更換網頁存取密碼").map_err(|m| ApiError(StatusCode::FORBIDDEN, m))?;
    let pw = body.password.trim().to_string();
    // 對外只有這一組密碼，短密碼等於沒有
    if !pw.is_empty() && pw.chars().count() < 8 {
        return Err(bad("網頁存取密碼至少需要 8 個字元"));
    }
    if pw.chars().count() > super::auth::PASSWORD_MAX_CHARS {
        return Err(bad(format!("網頁存取密碼最多 {} 個字元", super::auth::PASSWORD_MAX_CHARS)));
    }
    super::auth::set_password(&state.app.db, &pw).await?;
    event_log::log(
        &state.app.db,
        Level::Info,
        "security",
        "password",
        if pw.is_empty() { "網頁存取密碼已清除，外部連線將無法登入" } else { "網頁存取密碼已更新，已登入的連線都需要重新輸入" },
    );
    Ok(Json(serde_json::json!({ "ok": true, "password_set": !pw.is_empty() })))
}

#[derive(serde::Serialize, serde::Deserialize, sqlx::FromRow)]
struct ChuteApi {
    code: String,
    label: String,
    cid: i64,
    printer_port: Option<String>,
    enabled: bool,
    sort_order: i64,
}

async fn chutes_get(State(state): State<ServerState>) -> ApiResult<Vec<ChuteApi>> {
    let rows = sqlx::query_as::<_, ChuteApi>("SELECT code, label, cid, printer_port, enabled, sort_order FROM chutes ORDER BY sort_order, code")
        .fetch_all(&state.app.db)
        .await?;
    Ok(Json(rows))
}

// ---------- 異常件存證 ----------

#[derive(Deserialize)]
struct ReviewQuery {
    /// YYYY-MM-DD；空 = 今天
    #[serde(default)]
    day: String,
    /// sorter / middleware / noread / 空 = 全部
    #[serde(default)]
    kind: String,
    /// 只看讀碼失敗的某個原因（`db::abnormal_kind::NOREAD_CAUSES`）
    #[serde(default)]
    cause: String,
    #[serde(default = "default_review_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}
fn default_review_limit() -> i64 {
    24
}

#[derive(serde::Serialize, sqlx::FromRow)]
struct ReviewRow {
    id: i64,
    barcode: String,
    started_at: String,
    ended_ms: Option<i64>,
    status: i64,
    chute_code: Option<String>,
    chute_source: Option<String>,
    chute_reason: Option<String>,
    cart: Option<i64>,
    /// sorter / middleware / noread（規則在 `db::abnormal_kind`）
    kind: String,
    /// 讀碼失敗的系統判定原因；其他類別為 NULL
    cause: Option<String>,
    /// 這件的第一張讀碼站照片；NULL = 沒照片
    image_id: Option<i64>,
    has_orig: bool,
}

/// 某一天的異常件（分揀機異常、仲介機回傳、讀碼失敗）翻照片用：新的在前、分頁；
/// 各類件數與讀碼失敗各原因件數一律回整天的，不隨篩選變，頁首數字才穩
async fn abnormal_review(State(state): State<ServerState>, Query(q): Query<ReviewQuery>) -> ApiResult<serde_json::Value> {
    use crate::db::abnormal_kind::{NOREAD_CAUSES, SQL_ENDED_KIND, SQL_NOREAD_CAUSE};
    let day = super::stats::parse_day(Some(&q.day)).map_err(bad)?;
    let ts_from = format!("{} 00:00:00.000", day.format("%Y-%m-%d"));
    let ts_to = format!("{} 00:00:00.000", (day + chrono::Duration::days(1)).format("%Y-%m-%d"));
    if !matches!(q.kind.as_str(), "" | "sorter" | "middleware" | "noread") {
        return Err(bad("kind 只能是 sorter／middleware／noread 或空白"));
    }
    if !q.cause.is_empty() && !NOREAD_CAUSES.contains(&q.cause.as_str()) {
        return Err(bad("cause 不在原因清單裡"));
    }
    let default_chute = state.app.config.current().general.default_chute;
    // 篩選條件都放在外層，內層先把類別與原因算出來
    let base = format!(
        "SELECT p.*, {SQL_ENDED_KIND} AS kind, {SQL_NOREAD_CAUSE} AS cause,
                (SELECT MIN(i.id) FROM parcel_images i WHERE i.parcel_id = p.id) AS image_id,
                EXISTS (SELECT 1 FROM parcel_images i WHERE i.parcel_id = p.id AND i.orig_path IS NOT NULL) AS has_orig
           FROM parcels p
          WHERE p.started_at >= ? AND p.started_at < ? AND p.ended_ms IS NOT NULL"
    );
    let mut filter = String::from("kind IS NOT NULL");
    if !q.kind.is_empty() {
        filter.push_str(" AND kind = ?");
    }
    if !q.cause.is_empty() {
        filter.push_str(" AND cause = ?");
    }
    let list_sql = format!("SELECT * FROM ({base}) WHERE {filter} ORDER BY started_at DESC LIMIT ? OFFSET ?");
    let mut query = sqlx::query_as::<_, ReviewRow>(sqlx::AssertSqlSafe(list_sql)).bind(&default_chute).bind(&ts_from).bind(&ts_to);
    if !q.kind.is_empty() {
        query = query.bind(&q.kind);
    }
    if !q.cause.is_empty() {
        query = query.bind(&q.cause);
    }
    let rows = query.bind(q.limit.clamp(1, 200)).bind(q.offset.max(0)).fetch_all(&state.app.db).await?;

    let count_sql = format!("SELECT COUNT(*) FROM ({base}) WHERE {filter}");
    let mut cq = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql)).bind(&default_chute).bind(&ts_from).bind(&ts_to);
    if !q.kind.is_empty() {
        cq = cq.bind(&q.kind);
    }
    if !q.cause.is_empty() {
        cq = cq.bind(&q.cause);
    }
    let total: i64 = cq.fetch_one(&state.app.db).await?;

    // 整天的分類與原因統計（不受篩選影響）
    let summary_sql = format!("SELECT kind, cause, COUNT(*) AS n FROM ({base}) WHERE kind IS NOT NULL GROUP BY kind, cause");
    let summary: Vec<(String, Option<String>, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(summary_sql))
        .bind(&default_chute)
        .bind(&ts_from)
        .bind(&ts_to)
        .fetch_all(&state.app.db)
        .await?;
    let mut by_kind = serde_json::json!({ "sorter": 0, "middleware": 0, "noread": 0 });
    let mut cause_counts: std::collections::BTreeMap<&str, i64> = NOREAD_CAUSES.iter().map(|c| (*c, 0)).collect();
    let mut day_total = 0;
    for (kind, cause, n) in &summary {
        day_total += n;
        if let Some(v) = by_kind.get_mut(kind.as_str()) {
            *v = serde_json::json!(v.as_i64().unwrap_or(0) + n);
        }
        if let Some(c) = cause {
            if let Some(v) = cause_counts.get_mut(c.as_str()) {
                *v += n;
            }
        }
    }
    Ok(Json(serde_json::json!({
        "day": day.format("%Y-%m-%d").to_string(),
        "items": rows,
        "total": total,
        "day_total": day_total,
        "by_kind": by_kind,
        "cause_counts": NOREAD_CAUSES.iter().map(|c| serde_json::json!({ "cause": c, "count": cause_counts[c] })).collect::<Vec<_>>(),
    })))
}

async fn chutes_put(State(state): State<ServerState>, Json(list): Json<Vec<ChuteApi>>) -> ApiResult<serde_json::Value> {
    let default_code = state.app.config.current().general.default_chute;
    if !list.iter().any(|c| c.code == default_code) {
        return Err(bad(format!("格口表必須包含預設格口 {default_code}")));
    }
    for c in &list {
        if c.code.trim().is_empty() {
            return Err(bad("格口代號不可空白"));
        }
        if !crate::protocol::Cid(c.cid as u32).is_valid() {
            return Err(bad(format!("格口 {} 的 CID {} 格式不合法", c.code, c.cid)));
        }
    }
    let mut tx = state.app.db.begin().await?;
    sqlx::query("DELETE FROM chutes").execute(&mut *tx).await?;
    for c in &list {
        sqlx::query("INSERT INTO chutes (code, label, cid, printer_port, enabled, sort_order) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(c.code.trim())
            .bind(&c.label)
            .bind(c.cid)
            .bind(c.printer_port.as_ref().filter(|p| !p.trim().is_empty()))
            .bind(c.enabled as i64)
            .bind(c.sort_order)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    // 讓狀態機與解析器立即用新表
    let map: std::collections::HashMap<String, ChuteRow> = crate::tracker::load_chutes(&state.app.db).await?;
    if let Some(r) = state.app.resolver.get() {
        r.set_chutes(map);
    }
    // 狀態機經設定 watch 重載：只通知、不重寫設定檔
    state.app.config.touch();
    event_log::log(&state.app.db, Level::Info, "server", "chutes", String::from("格口對照已更新"));
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------- 裝置控制 ----------

async fn belt_start(State(state): State<ServerState>) -> ApiResult<serde_json::Value> {
    state.app.tracker.get().ok_or_else(|| unavailable("狀態機"))?.belt_start().await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn belt_stop(State(state): State<ServerState>) -> ApiResult<serde_json::Value> {
    state.app.tracker.get().ok_or_else(|| unavailable("狀態機"))?.belt_stop().await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn sorter_reset(State(state): State<ServerState>) -> ApiResult<serde_json::Value> {
    state.app.tracker.get().ok_or_else(|| unavailable("狀態機"))?.sorter_reset().await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
struct CommandBody {
    command: String,
}

/// 只允許查詢與燈控類指令直送，避免網頁誤下分揀指令
async fn sorter_command(State(state): State<ServerState>, Json(b): Json<CommandBody>) -> ApiResult<serde_json::Value> {
    let cmd = b.command.trim().to_string();
    let allowed = cmd.starts_with("KL ") || cmd.starts_with("Kd") || cmd.starts_with("p1 ") || cmd.starts_with("KY");
    if !allowed || cmd.contains('\n') {
        return Err(bad("只允許 KL／Kd／p1／KY 類指令"));
    }
    state.app.tracker.get().ok_or_else(|| unavailable("狀態機"))?.sorter_raw(cmd).await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
struct DeviceTestBody {
    /// belt / sorter / middleware
    target: String,
    /// 要測的位址（可帶尚未儲存的值）：`ip:port` 或中介機網址
    addr: String,
}

/// 設定頁「測試連線」：只做連線握手，不送任何指令，不影響正在跑的連線
async fn device_test(State(state): State<ServerState>, Json(b): Json<DeviceTestBody>) -> ApiResult<serde_json::Value> {
    let started = std::time::Instant::now();
    let result: Result<String, String> = match b.target.as_str() {
        "belt" | "sorter" => {
            let addr = b.addr.trim().to_string();
            match tokio::time::timeout(std::time::Duration::from_secs(2), tokio::net::TcpStream::connect(&addr)).await {
                Ok(Ok(_)) => Ok(format!("TCP 連線成功 {addr}")),
                Ok(Err(e)) => Err(format!("連線失敗：{e}")),
                Err(_) => Err("連線逾時（2 秒）".into()),
            }
        }
        "middleware" => {
            let base = b.addr.trim().trim_end_matches('/').to_string();
            let url = format!("{base}/healthz");
            let client = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(3)).build().map_err(|e| anyhow::anyhow!(e))?;
            match client.get(&url).timeout(std::time::Duration::from_secs(3)).send().await {
                Ok(r) if r.status().is_success() => Ok(format!("中介機回應 HTTP {}", r.status().as_u16())),
                Ok(r) => Err(format!("中介機回應 HTTP {}", r.status().as_u16())),
                Err(e) if e.is_timeout() => Err("連線逾時（3 秒）".into()),
                Err(e) => Err(format!("連線失敗：{e}")),
            }
        }
        _ => return Err(bad("target 只能是 belt／sorter／middleware")),
    };
    let ms = started.elapsed().as_millis() as u64;
    let _ = &state;
    Ok(Json(match result {
        Ok(msg) => serde_json::json!({ "ok": true, "ms": ms, "message": msg }),
        Err(msg) => serde_json::json!({ "ok": false, "ms": ms, "message": msg }),
    }))
}

// ---------- 列印 ----------

#[derive(serde::Serialize, sqlx::FromRow)]
struct PrintJobRow {
    id: i64,
    barcode: String,
    chute_code: String,
    printer_port: String,
    profile: Option<String>,
    status: String,
    attempts: i64,
    last_error: Option<String>,
    not_before_ms: i64,
    created_ms: i64,
    finished_ms: Option<i64>,
}

#[derive(Deserialize)]
struct StatusQuery {
    #[serde(default)]
    status: Option<String>,
    /// 關鍵字（條碼／格口／回報 ID／訊息，依端點而定）
    #[serde(default)]
    q: Option<String>,
    /// 條碼（可多筆：一筆模糊、多筆精確）
    #[serde(default)]
    barcode: Option<String>,
    #[serde(default)]
    chute: Option<String>,
    /// 本機時間 `YYYY-MM-DD HH:MM:SS[.mmm]`
    #[serde(default)]
    start: Option<String>,
    #[serde(default)]
    end: Option<String>,
    #[serde(default = "one")]
    page: i64,
    #[serde(default = "fifty")]
    limit: i64,
}
fn fifty() -> i64 {
    50
}

/// 分頁參數 → (limit, offset)
fn paging(page: i64, limit: i64) -> (i64, i64) {
    let limit = limit.clamp(1, 500);
    (limit, (page.max(1) - 1) * limit)
}

/// 本機時間字串 → epoch 毫秒；解析不了回 None（當作沒填）
fn local_ts_ms(s: Option<&str>) -> Option<i64> {
    let s = s?.trim();
    if s.is_empty() {
        return None;
    }
    for fmt in ["%Y-%m-%d %H:%M:%S%.3f", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%d"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return naive.and_local_timezone(chrono::Local).single().map(|t| t.timestamp_millis());
        }
        if fmt == "%Y-%m-%d" {
            if let Ok(d) = chrono::NaiveDate::parse_from_str(s, fmt) {
                return d.and_hms_opt(0, 0, 0).and_then(|n| n.and_local_timezone(chrono::Local).single()).map(|t| t.timestamp_millis());
            }
        }
    }
    None
}

/// 條碼欄位 → SQL 片段（一筆模糊、多筆精確）；沒填回 `1=1`
fn barcode_clause(col: &str, s: Option<&str>) -> (String, Vec<String>) {
    let tokens = s.map(split_tokens).unwrap_or_default();
    match tokens.len() {
        0 => ("1=1".into(), vec![]),
        1 => (format!("{col} LIKE ?"), vec![format!("%{}%", tokens[0])]),
        n => (format!("{col} IN ({})", vec!["?"; n].join(",")), tokens),
    }
}

/// 關鍵字（可多筆）→ `(col LIKE ? OR col2 LIKE ? OR …)` 每筆一組、彼此 OR；沒關鍵字回 `1=1`。
/// 回傳的 SQL 只含欄位名與 `?`，使用者輸入全在 binds。
fn kw_clause(cols: &[&str], q: Option<String>) -> (String, Vec<String>) {
    let tokens = q.map(|s| split_tokens(&s)).unwrap_or_default();
    if tokens.is_empty() {
        return ("1=1".into(), vec![]);
    }
    let mut parts = Vec::new();
    let mut binds = Vec::new();
    for t in tokens {
        let like = format!("%{t}%");
        let group: Vec<String> = cols.iter().map(|c| format!("{c} LIKE ?")).collect();
        parts.push(format!("({})", group.join(" OR ")));
        binds.extend(std::iter::repeat_n(like, cols.len()));
    }
    (format!("({})", parts.join(" OR ")), binds)
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.trim().is_empty())
}

async fn print_jobs(State(state): State<ServerState>, Query(q): Query<StatusQuery>) -> ApiResult<serde_json::Value> {
    let status = non_empty(q.status);
    let chute = non_empty(q.chute);
    let (start, end) = (local_ts_ms(q.start.as_deref()), local_ts_ms(q.end.as_deref()));
    let (bc_sql, bc_binds) = barcode_clause("barcode", q.barcode.as_deref());
    let (kw_sql, kw_binds) = kw_clause(&["barcode", "chute_code", "printer_port"], q.q);
    let (limit, offset) = paging(q.page, q.limit);
    let where_sql = format!(
        "WHERE (? IS NULL OR status = ?) AND (? IS NULL OR chute_code = ?) AND (? IS NULL OR created_ms >= ?) AND (? IS NULL OR created_ms <= ?) AND {bc_sql} AND {kw_sql}"
    );
    let mut count = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM print_jobs {where_sql}")))
        .bind(&status).bind(&status).bind(&chute).bind(&chute).bind(start).bind(start).bind(end).bind(end);
    for b in bc_binds.iter().chain(kw_binds.iter()) {
        count = count.bind(b);
    }
    let total = count.fetch_one(&state.app.db).await?;
    let mut list = sqlx::query_as::<_, PrintJobRow>(sqlx::AssertSqlSafe(format!(
        "SELECT id, barcode, chute_code, printer_port, profile, status, attempts, last_error, not_before_ms, created_ms, finished_ms
         FROM print_jobs {where_sql} ORDER BY id DESC LIMIT ? OFFSET ?"
    )))
    .bind(&status).bind(&status).bind(&chute).bind(&chute).bind(start).bind(start).bind(end).bind(end);
    for b in bc_binds.iter().chain(kw_binds.iter()) {
        list = list.bind(b);
    }
    let rows = list
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.app.db)
    .await?;
    Ok(Json(serde_json::json!({ "total": total, "list": rows })))
}

async fn print_job_retry(State(state): State<ServerState>, Path(id): Path<i64>) -> ApiResult<serde_json::Value> {
    state.app.printer.get().ok_or_else(|| unavailable("列印服務"))?.retry(id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// 補印前看「印出來長怎樣」：從還沒印掉的點陣檔還原成 PNG；印完檔案就清了，只有待印／失敗的看得到
async fn print_job_preview(State(state): State<ServerState>, Path(id): Path<i64>) -> Result<Response, ApiError> {
    let row: Option<(String, String)> = sqlx::query_as("SELECT tspl_path, status FROM print_jobs WHERE id = ?").bind(id).fetch_optional(&state.app.db).await?;
    let (path, status) = row.ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "找不到這筆列印任務".into()))?;
    let data = tokio::fs::read(&path).await.map_err(|_| {
        ApiError(StatusCode::NOT_FOUND, if status == "done" { "已印出的面單不保留點陣檔".into() } else { "點陣檔不存在".into() })
    })?;
    let png = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let raster = tspl::parse_bitmap(&data).ok_or_else(|| anyhow::anyhow!("點陣檔格式不對"))?;
        let img = crate::label::raster::to_preview(&raster);
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(img).write_to(&mut buf, image::ImageFormat::Png)?;
        Ok(buf.into_inner())
    })
    .await
    .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
    Ok(([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "no-store")], png).into_response())
}

async fn printers(State(state): State<ServerState>) -> ApiResult<serde_json::Value> {
    let attached: Vec<(String, String)> = match std::env::var("CIX_PRINT_FAKE_DIR") {
        Ok(dir) => std::fs::read_dir(&dir)
            .map(|rd| rd.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().to_string_lossy().into_owned())).collect())
            .unwrap_or_default(),
        Err(_) => usb::list_printers().into_iter().map(|(p, d)| (p, d.to_string_lossy().into_owned())).collect(),
    };
    let chutes = sqlx::query_as::<_, ChuteApi>("SELECT code, label, cid, printer_port, enabled, sort_order FROM chutes ORDER BY sort_order")
        .fetch_all(&state.app.db)
        .await?;
    let configured: Vec<serde_json::Value> = chutes
        .iter()
        .filter_map(|c| c.printer_port.as_ref().map(|p| (c, p)))
        .map(|(c, p)| serde_json::json!({ "chute": c.code, "port": p, "device": attached.iter().find(|(a, _)| a == p).map(|(_, d)| d.clone()), "online": attached.iter().any(|(a, _)| a == p) }))
        .collect();
    Ok(Json(serde_json::json!({ "attached": attached.iter().map(|(p, d)| serde_json::json!({ "port": p, "device": d })).collect::<Vec<_>>(), "configured": configured })))
}

async fn printer_test(State(state): State<ServerState>, Path(port): Path<String>) -> ApiResult<serde_json::Value> {
    let chute: Option<(String,)> = sqlx::query_as("SELECT code FROM chutes WHERE printer_port = ? LIMIT 1").bind(&port).fetch_optional(&state.app.db).await?;
    let label = chute.map(|c| c.0).unwrap_or_else(|| port.clone());
    state.app.printer.get().ok_or_else(|| unavailable("列印服務"))?.print_raw(&port, tspl::test_page(&label)).await?;
    event_log::log(&state.app.db, Level::Info, "printer", "test", format!("{label}（{port}）測試頁已送出"));
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------- 回報佇列 ----------

#[derive(serde::Serialize, sqlx::FromRow)]
struct ReportRow {
    id: i64,
    parcel_id: Option<i64>,
    barcode: Option<String>,
    chute_code: Option<String>,
    response_id: i64,
    status: String,
    retry_count: i64,
    next_retry_ms: i64,
    last_error: Option<String>,
    created_ms: i64,
    finished_ms: Option<i64>,
}

async fn report_queue(State(state): State<ServerState>, Query(q): Query<StatusQuery>) -> ApiResult<serde_json::Value> {
    let status = non_empty(q.status);
    let chute = non_empty(q.chute);
    let (start, end) = (local_ts_ms(q.start.as_deref()), local_ts_ms(q.end.as_deref()));
    let (bc_sql, bc_binds) = barcode_clause("p.barcode", q.barcode.as_deref());
    let (kw_sql, kw_binds) = kw_clause(&["p.barcode", "CAST(r.response_id AS TEXT)", "p.chute_code"], q.q);
    let (limit, offset) = paging(q.page, q.limit);
    let where_sql = format!(
        "WHERE (? IS NULL OR r.status = ?) AND (? IS NULL OR p.chute_code = ?) AND (? IS NULL OR r.created_ms >= ?) AND (? IS NULL OR r.created_ms <= ?) AND {bc_sql} AND {kw_sql}"
    );
    let mut count = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM report_queue r LEFT JOIN parcels p ON p.id = r.parcel_id {where_sql}"
    )))
    .bind(&status).bind(&status).bind(&chute).bind(&chute).bind(start).bind(start).bind(end).bind(end);
    for b in bc_binds.iter().chain(kw_binds.iter()) {
        count = count.bind(b);
    }
    let total = count.fetch_one(&state.app.db).await?;
    let mut list = sqlx::query_as::<_, ReportRow>(sqlx::AssertSqlSafe(format!(
        "SELECT r.id, r.parcel_id, p.barcode, p.chute_code, r.response_id, r.status, r.retry_count, r.next_retry_ms, r.last_error, r.created_ms, r.finished_ms
         FROM report_queue r LEFT JOIN parcels p ON p.id = r.parcel_id {where_sql} ORDER BY r.id DESC LIMIT ? OFFSET ?"
    )))
    .bind(&status).bind(&status).bind(&chute).bind(&chute).bind(start).bind(start).bind(end).bind(end);
    for b in bc_binds.iter().chain(kw_binds.iter()) {
        list = list.bind(b);
    }
    let rows = list
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.app.db)
    .await?;
    Ok(Json(serde_json::json!({ "total": total, "list": rows })))
}

async fn report_retry(State(state): State<ServerState>, Path(id): Path<i64>) -> ApiResult<serde_json::Value> {
    crate::middleware::report_queue::ReportQueue::new(state.app.db.clone()).retry(id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------- 日誌 ----------

#[derive(Deserialize)]
struct LogsQuery {
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    start: Option<String>,
    #[serde(default)]
    end: Option<String>,
    #[serde(default = "one")]
    page: i64,
    #[serde(default = "fifty")]
    limit: i64,
}

#[derive(serde::Serialize, sqlx::FromRow)]
struct LogRow {
    id: i64,
    level: String,
    category: String,
    action: String,
    message: String,
    created_at: String,
}

async fn logs(State(state): State<ServerState>, Query(q): Query<LogsQuery>) -> ApiResult<serde_json::Value> {
    let level = non_empty(q.level);
    let category = non_empty(q.category);
    let (start, end) = (non_empty(q.start), non_empty(q.end));
    let (kw_sql, kw_binds) = kw_clause(&["message", "action"], q.q);
    let (limit, offset) = paging(q.page, q.limit);
    let where_sql = format!(
        "WHERE (? IS NULL OR level = ?) AND (? IS NULL OR category = ?) AND (? IS NULL OR created_at >= ?) AND (? IS NULL OR created_at <= ?) AND {kw_sql}"
    );
    let mut count = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM event_log {where_sql}")))
        .bind(&level).bind(&level).bind(&category).bind(&category).bind(&start).bind(&start).bind(&end).bind(&end);
    for b in &kw_binds {
        count = count.bind(b);
    }
    let total = count.fetch_one(&state.app.db).await?;
    let mut list = sqlx::query_as::<_, LogRow>(sqlx::AssertSqlSafe(format!(
        "SELECT id, level, category, action, message, created_at FROM event_log {where_sql} ORDER BY id DESC LIMIT ? OFFSET ?"
    )))
    .bind(&level).bind(&level).bind(&category).bind(&category).bind(&start).bind(&start).bind(&end).bind(&end);
    for b in &kw_binds {
        list = list.bind(b);
    }
    let rows = list
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.app.db)
    .await?;
    Ok(Json(serde_json::json!({ "total": total, "list": rows })))
}

// ---------- 日誌檔（data/logs）----------

/// 只放行 `sorter.YYYY-MM-DD.log`／`signals-YYYY-MM-DD.log` 這種檔名，擋掉路徑穿越
fn is_log_file_name(name: &str) -> bool {
    let day_ok = |d: &str| d.len() == 10 && d.bytes().all(|b| b.is_ascii_digit() || b == b'-');
    name.strip_suffix(".log")
        .and_then(|n| n.strip_prefix("sorter.").or_else(|| n.strip_prefix("signals-")))
        .is_some_and(day_ok)
}

async fn log_files(State(state): State<ServerState>) -> ApiResult<Vec<serde_json::Value>> {
    let dir = state.app.data_dir.join("logs");
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !is_log_file_name(&name) {
                continue;
            }
            let meta = e.metadata().ok();
            let modified = meta.as_ref().and_then(|m| m.modified().ok()).map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M:%S").to_string());
            out.push(serde_json::json!({
                "name": name,
                "kind": if name.starts_with("signals-") { "signals" } else { "app" },
                "size": meta.map(|m| m.len()).unwrap_or(0),
                "modified": modified,
            }));
        }
    }
    out.sort_by(|a, b| b["name"].as_str().cmp(&a["name"].as_str()));
    Ok(Json(out))
}

async fn log_file_download(State(state): State<ServerState>, Path(name): Path<String>) -> Result<Response, ApiError> {
    if !is_log_file_name(&name) {
        return Err(bad("不是日誌檔名"));
    }
    let path = state.app.data_dir.join("logs").join(&name);
    let data = tokio::fs::read(&path).await.map_err(|_| ApiError(StatusCode::NOT_FOUND, "檔案不存在".into()))?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
        ],
        data,
    )
        .into_response())
}

// ---------- 手機遙控連線資訊 ----------

/// 本機區網 IPv4 與網頁埠：導覽列的「手機遙控」對話框拿來組網址與 QR
async fn lan_ips(State(state): State<ServerState>) -> ApiResult<serde_json::Value> {
    let bind = state.app.config.current().server.bind;
    let port = bind.rsplit(':').next().unwrap_or("18090").to_string();
    let mut ips = Vec::new();
    if let Ok(ifas) = local_ip_address::list_afinet_netifas() {
        for (name, ip) in ifas {
            if let std::net::IpAddr::V4(v4) = ip {
                if v4.is_loopback() || v4.is_link_local() || v4.is_unspecified() {
                    continue;
                }
                ips.push(serde_json::json!({ "name": name, "ip": v4.to_string() }));
            }
        }
    }
    Ok(Json(serde_json::json!({ "ips": ips, "port": port })))
}

// ---------- 光電檢查（IR） ----------

#[derive(serde::Serialize)]
struct IrDevice {
    /// 該台在集群內的編號 0–7（舊頁面顯示的 1000+m2）
    m2: u32,
    /// 0 沒回覆／未連線、1 正常、2 有光電被遮蔽
    status: u8,
}

/// `Kd[`：每台分揀機的光電總狀態；同時間只跑一個查詢
async fn ir_status(State(state): State<ServerState>) -> ApiResult<serde_json::Value> {
    let cfg = state.app.config.current();
    let tracker = state.app.tracker.get().ok_or_else(|| unavailable("狀態機"))?;
    let number = cfg.sorter.number.max(1) as usize;
    match tracker.ir_status().await {
        Ok(body) => {
            let statuses = crate::protocol::ir::parse_kd(&body, number);
            let devices: Vec<IrDevice> = statuses.into_iter().enumerate().map(|(i, s)| IrDevice { m2: i as u32, status: s }).collect();
            Ok(Json(serde_json::json!({ "devices": devices, "raw": body })))
        }
        Err(e) => Err(ApiError(StatusCode::SERVICE_UNAVAILABLE, e)),
    }
}

#[derive(Deserialize)]
struct IrDetailBody {
    m2: u32,
}

/// 單台每顆光電：`triggered[i]` = 第 i+1 顆此刻被遮蔽（讀值 < 1000）；`null` = 沒讀到
async fn ir_detail(State(state): State<ServerState>, Json(b): Json<IrDetailBody>) -> ApiResult<serde_json::Value> {
    let cfg = state.app.config.current();
    if b.m2 >= cfg.sorter.number.max(1) {
        return Err(bad("分揀機編號超出台數"));
    }
    let tracker = state.app.tracker.get().ok_or_else(|| unavailable("狀態機"))?;
    match tracker.ir_detail(b.m2).await {
        Ok(lines) => {
            let triggered = crate::protocol::ir::parse_p1(&lines, cfg.sorter.ir_num as usize);
            Ok(Json(serde_json::json!({ "m2": b.m2, "ir_num": cfg.sorter.ir_num, "triggered": triggered, "raw": lines })))
        }
        Err(e) => Err(ApiError(StatusCode::SERVICE_UNAVAILABLE, e)),
    }
}

#[derive(Deserialize)]
struct IrBlockBody {
    m2: u32,
    block: bool,
}

/// 屏蔽／解除屏蔽整台光電；會改變分揀機行為，要設定密碼
async fn ir_block(State(state): State<ServerState>, Json(b): Json<IrBlockBody>) -> ApiResult<serde_json::Value> {
    let cfg = state.app.config.current();
    if b.m2 >= cfg.sorter.number.max(1) {
        return Err(bad("分揀機編號超出台數"));
    }
    let tracker = state.app.tracker.get().ok_or_else(|| unavailable("狀態機"))?;
    tracker.ir_block(b.m2, b.block).await.map_err(|e| ApiError(StatusCode::SERVICE_UNAVAILABLE, e))?;
    event_log::log(&state.app.db, Level::Warn, "sorter", if b.block { "ir_block" } else { "ir_unblock" }, format!("分揀機 #{} 光電{}", b.m2 + 1, if b.block { "已屏蔽" } else { "已解除屏蔽" }));
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    #[test]
    fn 日誌檔名白名單() {
        assert!(super::is_log_file_name("sorter.2026-09-15.log"));
        assert!(super::is_log_file_name("signals-2026-09-15.log"));
        assert!(!super::is_log_file_name("../config.toml"));
        assert!(!super::is_log_file_name("sorter.2026-09-15.log.bak"));
        assert!(!super::is_log_file_name("signals-x.log"));
    }
}
