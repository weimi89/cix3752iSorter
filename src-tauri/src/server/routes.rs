//! REST 端點。回應一律 JSON，錯誤回 `{ "error": "..." }`。
//!
//! 會改變現場行為的操作（設定、格口表、重置分揀機）要帶 `X-Settings-Password`；
//! 皮帶啟停與重送任務不用（現場人員的日常操作）。

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
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

/// 需要設定密碼的操作
fn require_password(state: &ServerState, headers: &HeaderMap) -> Result<(), ApiError> {
    let expected = state.app.config.current().server.settings_password;
    let given = headers.get("x-settings-password").and_then(|v| v.to_str().ok()).unwrap_or("");
    if expected.is_empty() || given == expected {
        Ok(())
    } else {
        Err(ApiError(StatusCode::FORBIDDEN, "設定密碼錯誤".into()))
    }
}

pub(super) fn api_router() -> Router<ServerState> {
    Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/auth/check", post(auth_check))
        .route("/parcels", get(parcels_list))
        .route("/parcels/export.xlsx", get(parcels_export))
        .route("/parcels/{id}", get(parcel_detail))
        .route("/stats/daily", get(stats_daily))
        .route("/stats/hourly", get(stats_hourly))
        .route("/config", get(config_get).put(config_put))
        .route("/chutes", get(chutes_get).put(chutes_put))
        .route("/belt/start", post(belt_start))
        .route("/belt/stop", post(belt_stop))
        .route("/sorter/reset", post(sorter_reset))
        .route("/sorter/command", post(sorter_command))
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
        .route("/update/status", get(update_status))
        .route("/update/check", post(update_check))
        .route("/update/install", post(update_install))
        .route("/update/upload", post(update_upload))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

async fn status(State(state): State<ServerState>) -> ApiResult<serde_json::Value> {
    let cfg = state.app.config.current();
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
    Ok(Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "title": cfg.general.title,
        "machine": cfg.general.machine,
        "uptime_secs": state.app.started_at.elapsed().as_secs(),
        "devices": { "belt": rt.belt, "sorter": rt.sorter, "camera": rt.camera },
        "tracker": tracker,
        "chute_latency": state.app.resolver.get().map(|r| r.latency_stats()),
        "print": { "pending": print_pending, "failed": print_failed },
        "report": { "pending": report_pending, "failed": report_failed },
    })))
}

#[derive(Deserialize)]
struct AuthBody {
    password: String,
}

async fn auth_check(State(state): State<ServerState>, Json(b): Json<AuthBody>) -> ApiResult<serde_json::Value> {
    let ok = state.app.config.current().server.settings_password == b.password;
    Ok(Json(serde_json::json!({ "ok": ok })))
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

const PARCEL_COLS: &str = "id, ulid, barcode, chute_code, chute_cid, chute_source, status, belt_slot, cart, ir_length, gap, block_pos, lost_pos, response_id, started_at, started_ms, ended_ms, travel_ms";

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
    Ok(Json(serde_json::json!({ "parcel": parcel, "events": events, "print_jobs": prints })))
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
    abnormal: i64,
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
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT substr(started_at, 1, 13) AS h, COUNT(*), SUM(status = 3)
         FROM parcels WHERE started_at >= ? AND ended_ms IS NOT NULL GROUP BY h",
    )
    .bind(&since)
    .fetch_all(&state.app.db)
    .await?;
    let mut out: Vec<HourlyRow> = (0..hours)
        .map(|i| HourlyRow { hour: (first + Duration::hours(i)).format("%Y-%m-%d %H:00").to_string(), ..Default::default() })
        .collect();
    for (h, total, done) in rows {
        if let Some(b) = out.iter_mut().find(|b| b.hour.starts_with(&h)) {
            b.total = total;
            b.done = done;
            b.abnormal = total - done;
        }
    }
    Ok(Json(out))
}

// ---------- 設定 ----------

async fn config_get(State(state): State<ServerState>) -> ApiResult<AppConfig> {
    Ok(Json(state.app.config.current()))
}

async fn config_put(State(state): State<ServerState>, headers: HeaderMap, Json(cfg): Json<AppConfig>) -> ApiResult<serde_json::Value> {
    require_password(&state, &headers)?;
    for b in &cfg.emergency_buttons {
        if b.bit > 7 {
            return Err(bad(format!("按鈕「{}」的位元必須在 0–7", b.describe)));
        }
    }
    cfg.server.bind.parse::<std::net::SocketAddr>().map_err(|_| bad("網頁監聽位址格式錯誤"))?;
    state.app.config.update(cfg).await?;
    event_log::log(&state.app.db, Level::Info, "server", "config", String::from("設定已更新"));
    Ok(Json(serde_json::json!({ "ok": true })))
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

async fn chutes_put(State(state): State<ServerState>, headers: HeaderMap, Json(list): Json<Vec<ChuteApi>>) -> ApiResult<serde_json::Value> {
    require_password(&state, &headers)?;
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
    // 狀態機經設定 watch 重載（送一次相同設定即可觸發）
    let cfg = state.app.config.current();
    state.app.config.update(cfg).await?;
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

async fn sorter_reset(State(state): State<ServerState>, headers: HeaderMap) -> ApiResult<serde_json::Value> {
    require_password(&state, &headers)?;
    state.app.tracker.get().ok_or_else(|| unavailable("狀態機"))?.sorter_reset().await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
struct CommandBody {
    command: String,
}

/// 只允許查詢與燈控類指令直送，避免網頁誤下分揀指令
async fn sorter_command(State(state): State<ServerState>, headers: HeaderMap, Json(b): Json<CommandBody>) -> ApiResult<serde_json::Value> {
    require_password(&state, &headers)?;
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

// ---------- 自動更新 ----------

async fn update_status(State(state): State<ServerState>) -> ApiResult<serde_json::Value> {
    let up = state.app.updater.get().ok_or_else(|| unavailable("更新服務"))?;
    Ok(Json(serde_json::json!({ "current": crate::updater::CURRENT_VERSION, "busy": up.is_busy(), "last": up.last_check() })))
}

async fn update_check(State(state): State<ServerState>) -> ApiResult<crate::updater::UpdateInfo> {
    let up = state.app.updater.get().ok_or_else(|| unavailable("更新服務"))?;
    let info = up.check().await.map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("檢查更新失敗：{e}")))?;
    Ok(Json(info))
}

/// 下載並安裝：需設定密碼；成功後行程會在 1 秒後結束，交給 supervisor／systemd 重啟
async fn update_install(State(state): State<ServerState>, headers: HeaderMap) -> ApiResult<serde_json::Value> {
    require_password(&state, &headers)?;
    let up = state.app.updater.get().ok_or_else(|| unavailable("更新服務"))?.clone();
    up.download_and_install().await?;
    Ok(Json(serde_json::json!({ "ok": true, "restarting": true })))
}

/// 沒有外網時：網頁直接上傳發版的 tar.gz
async fn update_upload(State(state): State<ServerState>, headers: HeaderMap, body: axum::body::Bytes) -> ApiResult<serde_json::Value> {
    require_password(&state, &headers)?;
    if body.len() < 1024 {
        return Err(bad("檔案太小，不是發版的 tar.gz"));
    }
    let up = state.app.updater.get().ok_or_else(|| unavailable("更新服務"))?.clone();
    up.install_uploaded(&body).await?;
    Ok(Json(serde_json::json!({ "ok": true, "restarting": true })))
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
    let port = bind.rsplit(':').next().unwrap_or("8080").to_string();
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
