//! 網頁後台的存取控制。
//!
//! 這台工控機一旦做了路由器 Port Forward 就直接對外，這裡是唯一一道門：
//!
//! | 來源 | 待遇 |
//! |------|------|
//! | 內網網段 | 免登入，完整權限（現場電腦、桌面視窗、手機遙控） |
//! | 非內網 | 必須輸入共用密碼；通過後與坐在現場同等權限 |
//!
//! **外網來源不得修改存取控制本身**（`web_access` 與密碼）。少了這條，拿到共用密碼的人
//! 可以把 `lan_cidrs` 改成 `0.0.0.0/0`，之後全世界都被判定為內網，整道門形同虛設；
//! 或直接換掉密碼把真正的操作員永久鎖在門外。門鎖不能由門外的人來換。
//!
//! **來源判斷只認 TCP 連線的對端位址。** 這台機器直接對外、前面沒有反向代理，
//! `X-Forwarded-For` 之類的標頭是任何人都能自己填的，一旦拿來判斷內外網，
//! 外部只要送一行標頭就整道門直接失效。日後若真的擺了代理在前面，要先確認
//! 代理會覆寫該標頭，再改這裡 —— 不可為了「順手支援代理」就直接採信。

use std::net::{IpAddr, SocketAddr};

use argon2::{
    Argon2, PasswordHash,
    password_hash::{PasswordHasher, PasswordVerifier},
};
use axum::{
    Json,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use ipnet::IpNet;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;

use super::ServerState;
use crate::config::WebAccessConfig;
use crate::db::DbPool;
use crate::event_log::{self, Level};

/// 共用密碼的 argon2 雜湊存放位置（app_setting 的 key）
const PASSWORD_KEY: &str = "web_access_password_hash";

const COOKIE_NAME: &str = "sorter_web_session";

/// 需要登入才能碰的資料端點。
///
/// 靜態資源（HTML/JS/CSS、手機遙控頁的殼）不在此列：外網要先載得到登入頁才有辦法登入，
/// 而那些檔案只是 UI 程式碼，不含任何營運資料；資料一律經由這幾個前綴才拿得到。
fn needs_session(path: &str) -> bool {
    path.starts_with("/api/") || path.starts_with("/events/")
}

/// 桌面視窗與開發環境的來源。
///
/// 桌面模式的視窗載的是 `tauri://localhost`（Windows 為 `http://tauri.localhost`），
/// 再跨來源打本機的 HTTP 伺服器；開發時是 Vite 的 localhost。這幾個來源在 CORS 與
/// 跨站檢查都要放行，兩處共用同一份清單，否則會出現「CORS 過了、跨站檢查又擋」的錯位。
pub(super) fn is_trusted_origin(origin: &[u8]) -> bool {
    origin == b"tauri://localhost"
        || origin == b"http://tauri.localhost"
        || origin.starts_with(b"http://localhost:")
        || origin.starts_with(b"http://127.0.0.1:")
}

/// 是否為瀏覽器發起的跨站請求。
///
/// 內網來源完全靠 IP 放行、沒有帳號密碼，因此現場電腦只要開到一個惡意網頁，
/// 那個網頁就能對工控機送出請求：`fetch` 可以觸發 `POST /api/belt/stop` 這類
/// 不吃參數的指令，整條分揀線就停了。
///
/// 優先看 `Sec-Fetch-Site`：它由瀏覽器填、網頁改不掉，`fetch` 與 `<img>`／`<iframe>`
/// 發出的跨站請求都會帶。`same-origin` 是網頁自己的請求、`none` 是使用者直接開網址或
/// 掃 QR 進來的，兩者放行；其餘一律擋，但「使用者自己點連結進來的頂層導覽」除外 ——
/// 主管從 LINE 點網址進來就是這種，擋了連登入頁都看不到，而 CSRF 怕的是網頁在背景
/// 替使用者發請求，頂層導覽不會把結果回傳給原網站。
/// 舊瀏覽器沒有這個標頭時退回比對 `Origin` 與 `Host`。
/// 桌面視窗的來源（見 `is_trusted_origin`）先放行；`<img>` 這類子資源不帶 `Origin`，
/// 改看 `Referer` 的來源（瀏覽器填的，網頁可以不送但不能偽造成別人的）；
/// curl 這類非瀏覽器請求什麼標頭都不帶，不受影響。
fn is_cross_site(req: &Request) -> bool {
    let headers = req.headers();
    let origin = headers.get(header::ORIGIN).map(|v| v.as_bytes());

    if origin.is_some_and(is_trusted_origin) {
        return false;
    }
    if origin.is_none() && referer_origin(headers).is_some_and(|r| is_trusted_origin(r.as_bytes())) {
        return false;
    }

    if let Some(site) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        if site == "same-origin" || site == "none" {
            return false;
        }
        let dest = headers.get("sec-fetch-dest").and_then(|v| v.to_str().ok()).unwrap_or("");
        let mode = headers.get("sec-fetch-mode").and_then(|v| v.to_str().ok()).unwrap_or("");
        // 頂層導覽的豁免只給讀取：`<form method="POST">` 送出時標頭長得一模一樣，
        // 表單沒欄位時連內容型別檢查都過得了，不限定方法就等於留了一條停皮帶的後門
        let safe_method = matches!(*req.method(), axum::http::Method::GET | axum::http::Method::HEAD);
        if dest == "document" && mode == "navigate" && safe_method {
            return false;
        }
        return true;
    }

    let Some(origin) = origin.and_then(|o| std::str::from_utf8(o).ok()) else {
        return false;
    };
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or_default();
    // Origin 形如 `scheme://host[:port]`，去掉 scheme 後應與 Host 完全相同
    origin.split("://").nth(1).unwrap_or("") != host
}

/// 帶內容的寫入請求是否以 JSON 送出。
///
/// 要求 `application/json` 會讓瀏覽器對跨站請求先送預檢，而預檢只放行 `is_trusted_origin`
/// 的來源 —— 等於把上面那條 CSRF 路徑再堵一層。少了這道檢查的話，
/// `Content-Type: text/plain` 的「簡單請求」可以直接送達。
/// 沒有內容的 POST（皮帶啟停）不在此列，那種由跨站檢查負責。
fn write_body_is_json(req: &Request) -> bool {
    let headers = req.headers();
    let has_body = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|n| n > 0)
        || headers.contains_key(header::TRANSFER_ENCODING);
    if !has_body {
        return true;
    }
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        // 只認 type/subtype 恰為 application/json（後面可接 ;charset=…），`application/jsonp` 之類不算
        .is_some_and(|v| v.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("application/json"))
}

/// `Referer` 的來源部分（`scheme://host[:port]`，不含路徑）
fn referer_origin(headers: &HeaderMap) -> Option<String> {
    let r = headers.get(header::REFERER)?.to_str().ok()?;
    let (scheme, rest) = r.split_once("://")?;
    let host = rest.split('/').next()?;
    Some(format!("{scheme}://{host}"))
}

/// 來源是否落在設定的內網網段
pub(super) fn is_lan(ip: IpAddr, cidrs: &[String]) -> bool {
    // IPv4-mapped IPv6（::ffff:192.168.1.5）要還原成 IPv4 再比，
    // 否則雙堆疊監聽下內網電腦會被判成外網。
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    };

    // 本機永遠算內網，不受設定影響：桌面視窗、模擬器、手機遙控的 QR 都從本機打；
    // 網段清單被清空或打錯字時若連本機都擋，桌面版會整個壞掉，而使用者只看到「連不上」。
    if ip.is_loopback() {
        return true;
    }

    cidrs.iter().any(|c| match c.parse::<IpNet>() {
        Ok(net) => net.contains(&ip),
        // 設定打錯字時「不當成內網」：寧可多要一次密碼，
        // 也不要因為一個錯字讓整個外網被當成內網放行
        Err(_) => false,
    })
}

fn hash_token(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 產生 session token：兩個 v4 UUID 串接（各 122 bit 亂數），足以抵抗猜測
fn new_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

fn token_from_headers(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| k.trim() == COOKIE_NAME)
        .map(|(_, v)| v.trim().to_string())
}

fn cookie_token(req: &Request) -> Option<String> {
    token_from_headers(req.headers())
}

fn web_access_config(state: &ServerState) -> WebAccessConfig {
    state.app.config.current().web_access
}

// ── 密碼 ────────────────────────────────────────────────────────

pub(super) async fn stored_password_hash(db: &DbPool) -> anyhow::Result<Option<String>> {
    let row = sqlx::query("SELECT value FROM app_setting WHERE key = ?")
        .bind(PASSWORD_KEY)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| r.get::<String, _>("value")).filter(|s| !s.is_empty()))
}

/// 密碼長度上限（字元）。argon2 是慢雜湊，超長輸入等於讓外網免費燒 CPU；
/// 真人不會用這麼長的共用密碼。
pub(super) const PASSWORD_MAX_CHARS: usize = 128;

/// 設定共用密碼；空字串代表清除（外網從此無法登入）。
///
/// 與登入共用 `LOGIN_LOCK`，且「寫新雜湊」與「清所有 session」在同一筆交易：
/// 否則登入流程讀到舊雜湊、驗證通過後在清完 session 之後才插入新 session，
/// 那條用舊密碼換來的連線會一直活到自然到期，改密碼等於沒改。
pub(super) async fn set_password(db: &DbPool, plain: &str) -> anyhow::Result<()> {
    let hash = if plain.is_empty() {
        String::new()
    } else {
        // 鹽由 argon2 自己用系統亂數產生（16 bytes），輸出是標準 PHC 字串
        Argon2::default()
            .hash_password(plain.as_bytes())
            .map_err(|e| anyhow::anyhow!("密碼雜湊失敗: {e}"))?
            .to_string()
    };

    let _serialized = LOGIN_LOCK.lock().await;
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO app_setting (key, value, updated_at)
         VALUES (?, ?, datetime('now','localtime'))
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(PASSWORD_KEY)
    .bind(&hash)
    .execute(&mut *tx)
    .await?;
    // 改密碼等於要把所有人請出去重新驗證，否則舊密碼流出後對方仍能用既有連線
    sqlx::query("DELETE FROM web_session").execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

fn verify_password(plain: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default().verify_password(plain.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

// ── session ────────────────────────────────────────────────────

async fn session_valid(db: &DbPool, token: &str) -> bool {
    let hash = hash_token(token);
    let row = sqlx::query(
        "SELECT token_hash FROM web_session
         WHERE token_hash = ? AND expires_at > datetime('now','localtime')",
    )
    .bind(&hash)
    .fetch_optional(db)
    .await;

    match row {
        Ok(Some(_)) => {
            // 更新活動時間供稽核；失敗不影響本次放行
            let _ = sqlx::query("UPDATE web_session SET last_seen_at = datetime('now','localtime') WHERE token_hash = ?")
                .bind(&hash)
                .execute(db)
                .await;
            true
        }
        _ => false,
    }
}

async fn create_session(db: &DbPool, ip: &str, hours: u32) -> anyhow::Result<String> {
    // 順手清掉過期的，免得資料表無止盡長大
    let _ = sqlx::query("DELETE FROM web_session WHERE expires_at <= datetime('now','localtime')")
        .execute(db)
        .await;

    let token = new_token();
    sqlx::query(
        "INSERT INTO web_session (token_hash, client_ip, expires_at)
         VALUES (?, ?, datetime('now','localtime', ?))",
    )
    .bind(hash_token(&token))
    .bind(ip)
    .bind(format!("+{} hours", hours.max(1)))
    .execute(db)
    .await?;

    Ok(token)
}

// ── 失敗鎖定 ────────────────────────────────────────────────────

/// 登入流程的序列化鎖。
///
/// 「查有沒有被鎖 → 驗密碼 → 記一次失敗」是三個獨立步驟，中間沒有任何鎖。
/// 同一來源只要同時開 N 條連線，全部都會在**任何一次失敗被寫進資料庫之前**
/// 讀到「沒被鎖」，等於一口氣免費猜 N 次，`max_fail_attempts` 形同虛設。
/// 登入是低頻操作，argon2 驗證本身也要上百毫秒，序列化不會影響現場使用。
static LOGIN_LOCK: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

/// 這個來源還要被鎖多久（秒）；0 代表沒被鎖。
///
/// 查詢失敗時回 `Err` 而**不是當作沒被鎖** —— 若吞掉錯誤回 0，資料庫一忙碌
/// （SQLITE_BUSY）暴力破解保護就靜默失效，而且不會留下任何跡象。
async fn lock_remaining_secs(db: &DbPool, ip: &str) -> anyhow::Result<i64> {
    let row = sqlx::query(
        "SELECT CAST((julianday(locked_until) - julianday('now','localtime')) * 86400 AS INTEGER) AS secs
         FROM web_login_attempt
         WHERE client_ip = ? AND locked_until IS NOT NULL AND locked_until > datetime('now','localtime')",
    )
    .bind(ip)
    .fetch_optional(db)
    .await?;

    Ok(match row {
        Some(r) => r.try_get::<i64, _>("secs").unwrap_or(0).max(0),
        None => 0,
    })
}

/// 登入失敗計數用的來源鍵。
///
/// IPv6 一個人手上通常整個 /64 都是他的，照完整位址計數的話換一個位址就重新拿到 N 次機會，
/// 鎖定形同虛設；IPv4 沒有這個問題，照原位址。IPv4-mapped 的 IPv6 先還原成 IPv4。
fn lockout_key(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let seg = v6.segments();
                format!("{:x}:{:x}:{:x}:{:x}::/64", seg[0], seg[1], seg[2], seg[3])
            }
        },
    }
}

async fn record_fail(db: &DbPool, ip: &str, cfg: &WebAccessConfig) -> anyhow::Result<()> {
    // 順手清掉早就沒在鎖、也很久沒再失敗的紀錄，免得每個亂猜過一次的來源都永久留一列
    let _ = sqlx::query(
        "DELETE FROM web_login_attempt
         WHERE (locked_until IS NULL OR locked_until <= datetime('now','localtime'))
           AND (last_fail_at IS NULL OR last_fail_at < datetime('now','localtime','-1 day'))",
    )
    .execute(db)
    .await;

    sqlx::query(
        "INSERT INTO web_login_attempt (client_ip, fail_count, last_fail_at)
         VALUES (?, 1, datetime('now','localtime'))
         ON CONFLICT(client_ip) DO UPDATE SET
           fail_count = web_login_attempt.fail_count + 1,
           last_fail_at = datetime('now','localtime')",
    )
    .bind(ip)
    .execute(db)
    .await?;

    sqlx::query(
        "UPDATE web_login_attempt
         SET locked_until = datetime('now','localtime', ?), fail_count = 0
         WHERE client_ip = ? AND fail_count >= ?",
    )
    .bind(format!("+{} minutes", cfg.lock_minutes.max(1)))
    .bind(ip)
    .bind(cfg.max_fail_attempts.max(1) as i64)
    .execute(db)
    .await?;

    Ok(())
}

async fn clear_fails(db: &DbPool, ip: &str) {
    let _ = sqlx::query("DELETE FROM web_login_attempt WHERE client_ip = ?")
        .bind(ip)
        .execute(db)
        .await;
}

// ── 長連線（SSE） ────────────────────────────────────────────────

/// 取這條長連線的 session token（沒有就是空字串）
pub(super) fn stream_token(headers: &HeaderMap) -> String {
    token_from_headers(headers).unwrap_or_default()
}

/// 這條長連線現在還能不能繼續收事件。
///
/// SSE 一旦建立就不再經過中介層，所以要週期性回頭問這一次。**內外網也要每次重算** ——
/// 只在建立當下判斷的話，管理員事後把某個位址移出內網網段，那條已開著的串流會永遠
/// 繼續推送包裹資料；這與「外網 session 失效但串流還開著」是同一個問題的另一半。
pub(super) async fn stream_still_allowed(state: &ServerState, peer: IpAddr, token: &str) -> bool {
    let cfg = web_access_config(state);
    if is_lan(peer, &cfg.lan_cidrs) {
        return true;
    }
    cfg.enabled && !token.is_empty() && session_valid(&state.app.db, token).await
}

// ── 中介層 ──────────────────────────────────────────────────────

fn json_error(code: StatusCode, msg: &str) -> Response {
    (code, Json(serde_json::json!({ "error": msg }))).into_response()
}

/// 「此服務未對外開放」要讓前端分得出來：外網使用者登入中被管理員關掉開關時，
/// 所有請求都變 403，前端靠這個代碼把人帶回登入頁說明原因，而不是原地一直跳錯誤
fn not_public() -> Response {
    (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "此服務未對外開放", "code": "not_public" }))).into_response()
}

fn security_log(state: &ServerState, level: Level, action: &'static str, message: String) {
    event_log::log(&state.app.db, level, "security", action, message);
}

pub(super) async fn guard(
    State(state): State<ServerState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    // 直接讀當前設定，改設定即時生效，不必重啟 server
    let cfg = web_access_config(&state);
    let path = req.uri().path().to_string();
    let ip = peer.ip();

    // 這兩道要擋在「內網放行」之前 —— CSRF 針對的正是內網使用者的瀏覽器，
    // 先放行內網再檢查等於沒檢查。
    if is_cross_site(&req) {
        tracing::warn!(%ip, %path, "擋下跨站請求");
        return json_error(StatusCode::FORBIDDEN, "不接受跨站請求");
    }
    if !matches!(*req.method(), axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS)
        && !write_body_is_json(&req)
    {
        return json_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "請求內容必須以 application/json 送出");
    }

    if is_lan(ip, &cfg.lan_cidrs) {
        return next.run(req).await;
    }

    // ── 以下都是外網來源 ──

    if !cfg.enabled {
        // 對外沒開放時完全不透露這裡有什麼，連登入頁都不給
        return not_public();
    }

    if !needs_session(&path) {
        // 靜態資源與登入端點：讓外網載得到登入頁
        return next.run(req).await;
    }

    let ok = match cookie_token(&req) {
        Some(t) => session_valid(&state.app.db, &t).await,
        None => false,
    };

    if ok {
        next.run(req).await
    } else {
        json_error(StatusCode::UNAUTHORIZED, "尚未登入或登入已逾期")
    }
}

/// 只准在工控機本機或現場網路內做的事（更換共用密碼、改存取控制本身）。
///
/// 換密碼等同換門鎖，而且**不需要輸入舊密碼**；這一期沒有 TLS，對外那段路徑的
/// session cookie 是明文 —— 攔到 cookie 的人若還能換密碼，就不只是「偷看到資料」，
/// 而是直接把真正的操作員永久鎖在門外（單純偷 session 會隨到期失效，換掉密碼不會）。
pub(super) fn guard_lan_only(state: &ServerState, peer: IpAddr, what: &str) -> Result<(), String> {
    let cfg = web_access_config(state);
    if is_lan(peer, &cfg.lan_cidrs) {
        return Ok(());
    }
    tracing::warn!(%peer, %what, "外網來源嘗試進行僅限內網的操作，已拒絕");
    security_log(state, Level::Warn, "lan_only_denied", format!("外網 {peer} 嘗試「{what}」被拒"));
    Err(format!("{what}只能在工控機本機或現場網路內進行"))
}

/// 外網來源送來的整份設定裡，`web_access` 不得與現行不同（理由見模組說明）。
///
/// 回 `Ok(true)` 代表來源是外網：呼叫端寫入時仍要以現況覆蓋 `web_access`，
/// 這裡的比對只是為了把「想改」的嘗試記下來，不是寫入時的依據。
pub(super) fn guard_web_access_change(
    state: &ServerState,
    peer: IpAddr,
    incoming: &WebAccessConfig,
) -> Result<bool, String> {
    let cfg = web_access_config(state);
    if is_lan(peer, &cfg.lan_cidrs) {
        return Ok(false);
    }
    if *incoming != cfg {
        guard_lan_only(state, peer, "修改網頁存取設定")?;
    }
    Ok(true)
}

// ── 登入 / 登出 / 狀態 ──────────────────────────────────────────

#[derive(Deserialize)]
pub(super) struct LoginBody {
    password: String,
}

#[derive(Serialize)]
pub(super) struct AuthStatus {
    /// 這條連線目前能不能存取資料端點
    authenticated: bool,
    /// 來源是否被視為內網（內網免登入）
    lan: bool,
    /// 是否已設定共用密碼 —— 沒設定的話外網永遠進不來
    password_set: bool,
    /// 對外存取總開關
    enabled: bool,
}

pub(super) async fn status(
    State(state): State<ServerState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
) -> Response {
    let cfg = web_access_config(&state);
    let lan = is_lan(peer.ip(), &cfg.lan_cidrs);
    let password_set = stored_password_hash(&state.app.db).await.ok().flatten().is_some();

    let authenticated = if lan {
        true
    } else {
        match cookie_token(&req) {
            Some(t) => session_valid(&state.app.db, &t).await,
            None => false,
        }
    };

    Json(AuthStatus { authenticated, lan, password_set, enabled: cfg.enabled }).into_response()
}

pub(super) async fn login(
    State(state): State<ServerState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<LoginBody>,
) -> Response {
    let cfg = web_access_config(&state);
    let ip = lockout_key(peer.ip());

    if !cfg.enabled && !is_lan(peer.ip(), &cfg.lan_cidrs) {
        return not_public();
    }
    if body.password.chars().count() > PASSWORD_MAX_CHARS {
        return json_error(StatusCode::BAD_REQUEST, "密碼太長");
    }

    // 從這裡到「記錄失敗」為止必須不可分割，理由見 LOGIN_LOCK
    let _serialized = LOGIN_LOCK.lock().await;

    let wait = match lock_remaining_secs(&state.app.db, &ip).await {
        Ok(w) => w,
        Err(e) => {
            // 查不出鎖定狀態時寧可擋下來：放行等於在資料庫出問題的期間開放無限次嘗試
            tracing::warn!(?e, %ip, "查詢登入鎖定狀態失敗，本次登入一併拒絕");
            return json_error(StatusCode::SERVICE_UNAVAILABLE, "目前無法驗證登入，請稍候再試");
        }
    };
    if wait > 0 {
        return json_error(
            StatusCode::TOO_MANY_REQUESTS,
            &format!("嘗試次數過多，請於 {} 分鐘後再試", wait.div_euclid(60) + 1),
        );
    }

    let stored = match stored_password_hash(&state.app.db).await {
        Ok(Some(h)) => h,
        // 沒設密碼時外網一律進不來 —— 「沒設密碼就放行」會讓剛裝好、
        // 還沒設定的機器在對外開關打開的瞬間門戶大開
        _ => return json_error(StatusCode::FORBIDDEN, "尚未設定網頁存取密碼，請先在工控機的系統設定裡設定"),
    };

    if !verify_password(&body.password, &stored) {
        if let Err(e) = record_fail(&state.app.db, &ip, &cfg).await {
            // 記不起來就等於這次失敗沒被計數，鎖定會比預期晚生效 —— 至少要留下痕跡
            tracing::warn!(?e, %ip, "記錄登入失敗次數失敗，鎖定計數可能不準");
        }
        security_log(&state, Level::Warn, "login_failed", format!("來源 {ip} 密碼錯誤"));
        return json_error(StatusCode::UNAUTHORIZED, "密碼錯誤");
    }

    clear_fails(&state.app.db, &ip).await;

    let token = match create_session(&state.app.db, &ip, cfg.session_hours).await {
        Ok(t) => t,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };

    security_log(&state, Level::Info, "login", format!("來源 {ip} 登入成功"));

    // 尚未啟用 TLS，因此不加 Secure —— 加了的話 http 連線會直接收不到這個 cookie，
    // 整個登入功能等於不能用。**改用 https 之後這裡要補上 Secure。**
    let cookie = format!(
        "{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}",
        cfg.session_hours.max(1) as u64 * 3600
    );

    (StatusCode::OK, [(header::SET_COOKIE, cookie)], Json(serde_json::json!({ "ok": true }))).into_response()
}

pub(super) async fn logout(State(state): State<ServerState>, req: Request) -> Response {
    if let Some(t) = cookie_token(&req) {
        let _ = sqlx::query("DELETE FROM web_session WHERE token_hash = ?")
            .bind(hash_token(&t))
            .execute(&state.app.db)
            .await;
    }

    let cookie = format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0");
    (StatusCode::OK, [(header::SET_COOKIE, cookie)], Json(serde_json::json!({ "ok": true }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cidrs() -> Vec<String> {
        WebAccessConfig::default().lan_cidrs
    }

    #[test]
    fn 內網網段判定() {
        for ip in ["192.168.177.5", "10.0.0.5", "172.16.3.9", "127.0.0.1"] {
            assert!(is_lan(ip.parse().unwrap(), &cidrs()), "{ip} 應視為內網");
        }
    }

    #[test]
    fn 外網位址不得被當成內網() {
        // 172.32 落在 172.16/12 之外，是常見的邊界誤判
        for ip in ["8.8.8.8", "1.1.1.1", "172.32.0.1", "11.0.0.1"] {
            assert!(!is_lan(ip.parse().unwrap(), &cidrs()), "{ip} 不該視為內網");
        }
    }

    #[test]
    fn ipv4_mapped_的內網位址要還原後再判() {
        let ip: IpAddr = "::ffff:192.168.1.20".parse().unwrap();
        assert!(is_lan(ip, &cidrs()));
    }

    #[test]
    fn 本機永遠算內網_即使網段清單是空的() {
        assert!(is_lan("127.0.0.1".parse().unwrap(), &[]));
        assert!(is_lan("::1".parse().unwrap(), &[]));
    }

    #[test]
    fn 網段設定打錯字時不放行() {
        let bad = vec!["19.2.168.0/999".to_string(), "not-a-cidr".to_string()];
        assert!(!is_lan("192.168.1.1".parse().unwrap(), &bad));
    }

    fn req_with(method: &str, headers: &[(&str, &str)]) -> Request {
        let mut b = Request::builder().method(method).uri("/api/belt/stop");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(axum::body::Body::empty()).unwrap()
    }

    #[test]
    fn 同源請求不算跨站() {
        assert!(!is_cross_site(&req_with("POST", &[("origin", "http://192.168.177.5:18090"), ("host", "192.168.177.5:18090")])));
        assert!(!is_cross_site(&req_with("POST", &[("sec-fetch-site", "same-origin")])));
        assert!(!is_cross_site(&req_with("GET", &[("sec-fetch-site", "none")])));
    }

    #[test]
    fn 桌面視窗與開發環境的來源要放行() {
        // 桌面模式：視窗是 tauri://localhost，跨來源打本機伺服器，瀏覽器引擎會標成 cross-site
        for origin in ["tauri://localhost", "http://tauri.localhost", "http://localhost:5180", "http://127.0.0.1:5180"] {
            assert!(
                !is_cross_site(&req_with("POST", &[("origin", origin), ("sec-fetch-site", "cross-site"), ("host", "127.0.0.1:18090")])),
                "{origin} 應放行"
            );
        }
    }

    #[test]
    fn 惡意網站的跨站請求要擋掉() {
        assert!(is_cross_site(&req_with("POST", &[("origin", "https://evil.example.com"), ("host", "192.168.177.5:18090")])));
        // 同主機不同埠也算跨站
        assert!(is_cross_site(&req_with("POST", &[("origin", "http://192.168.177.5:9999"), ("host", "192.168.177.5:18090")])));
        // <img> 不帶 Origin，但會帶 Sec-Fetch-Site
        assert!(is_cross_site(&req_with("GET", &[("sec-fetch-site", "cross-site"), ("sec-fetch-dest", "image")])));
        assert!(is_cross_site(&req_with("GET", &[("sec-fetch-site", "same-site")])));
    }

    #[test]
    fn 桌面視窗的圖片子資源不帶_origin_靠_referer_放行() {
        // <img src="http://127.0.0.1:18090/api/print-jobs/1/preview.png">：no-cors 子資源不帶 Origin
        assert!(!is_cross_site(&req_with("GET", &[("referer", "tauri://localhost/"), ("sec-fetch-site", "cross-site"), ("sec-fetch-dest", "image")])));
        assert!(!is_cross_site(&req_with("GET", &[("referer", "http://localhost:5180/index.html"), ("sec-fetch-site", "cross-site"), ("sec-fetch-dest", "image")])));
        // 惡意網頁的 Referer 是它自己，仍要擋
        assert!(is_cross_site(&req_with("GET", &[("referer", "https://evil.example.com/"), ("sec-fetch-site", "cross-site"), ("sec-fetch-dest", "image")])));
        // Referer 只在沒有 Origin 時才看：帶了不受信任的 Origin 就不能靠 Referer 翻案
        assert!(is_cross_site(&req_with("POST", &[("origin", "https://evil.example.com"), ("referer", "tauri://localhost/"), ("sec-fetch-site", "cross-site")])));
    }

    #[test]
    fn 跨站表單_post_不得借頂層導覽的豁免() {
        // <form method="POST" action="http://工控機/api/belt/stop"> 送出時的標頭
        assert!(is_cross_site(&req_with("POST", &[("sec-fetch-site", "cross-site"), ("sec-fetch-dest", "document"), ("sec-fetch-mode", "navigate")])));
    }

    #[test]
    fn 從聊天軟體點連結進來要開得起來_但子資源不可冒用() {
        assert!(!is_cross_site(&req_with("GET", &[("sec-fetch-site", "cross-site"), ("sec-fetch-dest", "document"), ("sec-fetch-mode", "navigate")])));
        assert!(is_cross_site(&req_with("GET", &[("sec-fetch-site", "cross-site"), ("sec-fetch-dest", "iframe"), ("sec-fetch-mode", "navigate")])));
        assert!(is_cross_site(&req_with("POST", &[("sec-fetch-site", "cross-site"), ("sec-fetch-dest", "empty"), ("sec-fetch-mode", "cors")])));
    }

    #[test]
    fn 非瀏覽器請求不帶標頭_不受影響() {
        assert!(!is_cross_site(&req_with("POST", &[("host", "192.168.177.5:18090")])));
    }

    #[test]
    fn 帶內容的寫入只收_json_沒內容的照常() {
        assert!(write_body_is_json(&req_with("POST", &[])));
        assert!(write_body_is_json(&req_with("POST", &[("content-length", "0")])));
        assert!(write_body_is_json(&req_with("PUT", &[("content-length", "12"), ("content-type", "application/json; charset=utf-8")])));
        // 這兩種是「簡單請求」，不觸發預檢，正是 CSRF 會用的送法
        assert!(!write_body_is_json(&req_with("POST", &[("content-length", "12"), ("content-type", "text/plain")])));
        assert!(!write_body_is_json(&req_with("POST", &[("content-length", "12"), ("content-type", "application/x-www-form-urlencoded")])));
        assert!(!write_body_is_json(&req_with("POST", &[("content-length", "12")])));
    }

    #[test]
    fn 內容型別只認_application_json_本身() {
        assert!(write_body_is_json(&req_with("POST", &[("content-length", "3"), ("content-type", "Application/JSON")])));
        assert!(!write_body_is_json(&req_with("POST", &[("content-length", "3"), ("content-type", "application/jsonp")])));
        assert!(!write_body_is_json(&req_with("POST", &[("content-length", "3"), ("content-type", "application/json-patch+json")])));
    }

    #[test]
    fn 鎖定鍵_ipv6_以_64_為單位_ipv4_照原位址() {
        assert_eq!(lockout_key("203.0.113.9".parse().unwrap()), "203.0.113.9");
        assert_eq!(lockout_key("::ffff:203.0.113.9".parse().unwrap()), "203.0.113.9");
        let a = lockout_key("2001:db8:1:2:aaaa::1".parse().unwrap());
        let b = lockout_key("2001:db8:1:2:bbbb::2".parse().unwrap());
        assert_eq!(a, b, "同一個 /64 要算同一個來源");
        assert_ne!(a, lockout_key("2001:db8:1:3::1".parse().unwrap()));
    }

    #[test]
    fn 需要登入的路徑() {
        for p in ["/api/status", "/api/belt/stop", "/events/stream"] {
            assert!(needs_session(p), "{p}");
        }
        for p in ["/", "/index.html", "/assets/app.js", "/control", "/auth/status", "/auth/login"] {
            assert!(!needs_session(p), "{p}");
        }
    }

    #[test]
    fn cookie_解析() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, "other=1; sorter_web_session=abc123; x=y".parse().unwrap());
        assert_eq!(token_from_headers(&h).as_deref(), Some("abc123"));
        let mut none = HeaderMap::new();
        none.insert(header::COOKIE, "other=1".parse().unwrap());
        assert!(token_from_headers(&none).is_none());
    }

    #[test]
    fn 密碼雜湊可驗證_且格式為_phc() {
        let h = Argon2::default().hash_password(b"pa55word").unwrap().to_string();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("pa55word", &h));
        assert!(!verify_password("wrong", &h));
        assert!(!verify_password("pa55word", "not-a-hash"));
    }

    #[test]
    fn token_雜湊為_sha256_小寫十六進位() {
        assert_eq!(hash_token("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(new_token().len(), 64);
    }
}
