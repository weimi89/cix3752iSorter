//! 格口解析：條碼 → 中介機 → `channel_code` → `chutes` 表 → CID，結果送回狀態機。
//!
//! 每個請求獨立 task，慢回應不會卡住其他件；狀態機在 `~O` 時若還沒答案就走預設口，
//! 之後才到的答案只留紀錄（見 `machine.rs on_chute`）。
//!
//! 有面單的件要**先把面單抓下來**才算有答案（舊 Node 版同樣做法）：抓不到就整件改走預設口、
//! 不回報、不印——寧可到異常口由人工處理，也不要包裹到了正常格口卻沒有面單可貼。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::AppState;
use crate::event_bus;
use crate::event_log::{self, Level};
use crate::middleware::{Middleware, ParcelResp};
use crate::protocol::Cid;
use crate::tracker::{ChuteRequest, ChuteRow, ChuteSource, LabelPayload, TrackerHandle};

/// 中介機給的面單來源，決定後才去抓
#[derive(Clone, Debug, serde::Serialize)]
pub struct LabelInfo {
    pub label_path: String,
    pub print_profile: Option<String>,
    pub is_error_label: bool,
}

#[derive(Clone)]
pub struct ChuteResolver {
    app: AppState,
    mw: Middleware,
    tracker: TrackerHandle,
    chutes: Arc<RwLock<HashMap<String, ChuteRow>>>,
    latency: Arc<RwLock<LatencyWindow>>,
}

/// 最近 N 次格口查詢（含面單下載）的耗時，看板用：中介機變慢時現場能提早察覺
pub struct LatencyWindow {
    samples: std::collections::VecDeque<(i64, u32)>,
    cap: usize,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct LatencyStats {
    pub samples: usize,
    pub p50_ms: u32,
    pub p90_ms: u32,
    pub p99_ms: u32,
    pub max_ms: u32,
    /// 最近一小時內超過 `budget_ms` 的比例（%）
    pub over_budget_pct: f32,
    pub budget_ms: u32,
}

impl LatencyWindow {
    pub fn new(cap: usize) -> Self {
        Self { samples: std::collections::VecDeque::with_capacity(cap), cap }
    }

    pub fn push(&mut self, ts_ms: i64, elapsed_ms: u32) {
        if self.samples.len() == self.cap {
            self.samples.pop_front();
        }
        self.samples.push_back((ts_ms, elapsed_ms));
    }

    /// 只看最近 `window_ms` 內的樣本
    pub fn stats(&self, now_ms: i64, window_ms: i64, budget_ms: u32) -> LatencyStats {
        let mut v: Vec<u32> = self.samples.iter().filter(|(t, _)| now_ms - t <= window_ms).map(|(_, e)| *e).collect();
        if v.is_empty() {
            return LatencyStats { budget_ms, ..Default::default() };
        }
        v.sort_unstable();
        let pick = |q: f64| v[((v.len() as f64 - 1.0) * q).round() as usize];
        let over = v.iter().filter(|&&e| e > budget_ms).count();
        LatencyStats {
            samples: v.len(),
            p50_ms: pick(0.5),
            p90_ms: pick(0.9),
            p99_ms: pick(0.99),
            max_ms: *v.last().unwrap(),
            over_budget_pct: over as f32 * 100.0 / v.len() as f32,
            budget_ms,
        }
    }
}

pub struct Decision {
    pub code: String,
    pub cid: Cid,
    pub source: ChuteSource,
    pub response_id: Option<i64>,
    pub label: Option<LabelInfo>,
    pub note: Option<String>,
    /// 統計用的原因代碼（見 migration 0003）；正常給格口為 None
    pub reason: Option<String>,
}

/// 今天同一條碼的一筆前科（`history_verdict` 的輸入，從 parcels 表撈）
#[derive(Debug, Clone)]
pub struct PriorParcel {
    pub status: i64,
    pub chute_code: Option<String>,
    pub chute_source: Option<String>,
    pub chute_reason: Option<String>,
    pub started_ms: i64,
}

impl PriorParcel {
    /// 上次真的分到正常格口並完成：走異常口的件狀態也是「完成」，那是重投迴圈的常態，不算
    fn completed_normally(&self) -> bool {
        self.status == 3 && self.chute_reason.is_none() && self.decided_normally()
    }

    /// 中介機正常給過格口（不論有沒有走完——幾秒前才分的件多半還在路上）
    fn decided_normally(&self) -> bool {
        self.chute_reason.is_none() && matches!(self.chute_source.as_deref(), Some("api") | Some("manual"))
    }
}

/// 純函式：同一條碼 `hold_ms` 內剛正常分過又進線 → 這件多半是讀到鄰件的條碼（相機視野含進料區），
/// 回那筆前科的上線時間；`hold_ms` ≤ 0 關閉。`prior` 只含 `hold_ms` 內的件
pub fn reentry_hold(prior: &[PriorParcel], hold_ms: i64) -> Option<i64> {
    if hold_ms <= 0 {
        return None;
    }
    prior.iter().find(|p| p.decided_normally()).map(|p| p.started_ms)
}

/// 查資料庫版：回 `(前科上線時間, 現在)`；`hold_ms` ≤ 0 不查
pub async fn check_reentry_hold(db: &crate::db::DbPool, barcode: &str, ulid: &str, hold_ms: i64) -> Option<(i64, i64)> {
    if hold_ms <= 0 {
        return None;
    }
    let now = crate::db::now_ms();
    let prior = load_prior(db, barcode, ulid, now - hold_ms).await;
    reentry_hold(&prior, hold_ms).map(|prev_ms| (prev_ms, now))
}

#[derive(Debug, Clone, PartialEq)]
pub enum HistoryAlert {
    /// 這件又因同一原因走異常口；`count` 含這一次
    RepeatDefault { count: usize, reason: String },
    /// 之前已完成分揀，現在又進線
    ReEntry { prev_chute: String, prev_started_ms: i64 },
}

/// 同一條碼從 `since_ms` 起的其他包裹（排除自己），新的在前。查詢失敗回空——這是提示，不能影響分揀
pub async fn load_prior(db: &crate::db::DbPool, barcode: &str, ulid: &str, since_ms: i64) -> Vec<PriorParcel> {
    let rows: Vec<(i64, Option<String>, Option<String>, Option<String>, i64)> = sqlx::query_as(
        "SELECT status, chute_code, chute_source, chute_reason, started_ms FROM parcels
          WHERE barcode = ? AND ulid <> ? AND started_ms >= ? ORDER BY started_ms DESC LIMIT 10",
    )
    .bind(barcode)
    .bind(ulid)
    .bind(since_ms)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .map(|(status, chute_code, chute_source, chute_reason, started_ms)| PriorParcel { status, chute_code, chute_source, chute_reason, started_ms })
        .collect()
}

/// 純函式：依前科決定要不要提示。同原因重複走異常口優先於「已完成又進線」——
/// 關轉件每次都是異常口，若先看「曾完成」永遠不會命中；反過來一件完成過的包裹再進線後走異常口，
/// 也是值得提示「又來了」而不是「格口滿」
pub fn history_verdict(prior: &[PriorParcel], source: ChuteSource, reason: Option<&str>) -> Option<HistoryAlert> {
    if prior.is_empty() {
        return None;
    }
    if source == ChuteSource::Default {
        if let Some(reason) = reason {
            let same = prior.iter().filter(|p| p.chute_reason.as_deref() == Some(reason)).count();
            if same >= 1 {
                return Some(HistoryAlert::RepeatDefault { count: same + 1, reason: reason.to_string() });
            }
        }
    }
    prior
        .iter()
        .find(|p| p.completed_normally())
        .map(|p| HistoryAlert::ReEntry { prev_chute: p.chute_code.clone().unwrap_or_else(|| "?".into()), prev_started_ms: p.started_ms })
}

/// 原因代碼的現場說法（提示訊息與語音用；統計頁另有 i18n）
pub fn reason_label(code: &str) -> &str {
    match code {
        "STORE_CLOSED" => "門市關轉",
        "REENTRY" => "同碼短時間再進線",
        "NOT_FOUND" => "查無訂單",
        "UNCONFIRMED" => "訂單未確認",
        "NO_CHANNEL" => "中介機未給格口",
        "CHUTE_DISABLED" => "格口已停用",
        "CHUTE_UNKNOWN" => "格口不在對照表",
        "MW_UNREACHABLE" => "中介機連不上",
        "LABEL_FETCH_FAILED" => "面單下載失敗",
        other => other,
    }
}

/// 純函式：把中介機回應對到格口表（可單元測試）
pub fn decide(resp: &ParcelResp, chutes: &HashMap<String, ChuteRow>, default_code: &str) -> Decision {
    let default_cid = chutes.get(default_code).map(|r| r.cid).unwrap_or(Cid(1007301));
    let label = resp.label_path.as_ref().map(|p| LabelInfo {
        label_path: p.clone(),
        print_profile: resp.print_profile.clone(),
        is_error_label: resp.is_error_label,
    });

    // 中介機的錯誤碼原樣當原因代碼（大寫），兩邊統計才對得起來
    let error_code = resp.error_code.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_uppercase);
    if error_code.as_deref() == Some("NOREAD") {
        return Decision { code: default_code.into(), cid: default_cid, source: ChuteSource::NoRead, response_id: None, label: None, note: None, reason: Some("NOREAD".into()) };
    }
    match resp.channel_code.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(code) => match chutes.get(code) {
            // 有格口但帶錯誤碼 = 錯誤提示面單：包裹照分，原因記下來
            Some(row) if row.enabled => Decision {
                code: row.code.clone(),
                cid: row.cid,
                source: ChuteSource::Api,
                response_id: resp.response_id,
                label,
                note: resp.error_code.clone().map(|e| format!("{e}: {}", resp.message.clone().unwrap_or_default())),
                reason: error_code,
            },
            Some(_) => Decision {
                code: default_code.into(),
                cid: default_cid,
                source: ChuteSource::Default,
                response_id: resp.response_id,
                label,
                note: Some(format!("格口 {code} 已停用")),
                reason: Some("CHUTE_DISABLED".into()),
            },
            None => Decision {
                code: default_code.into(),
                cid: default_cid,
                source: ChuteSource::Default,
                response_id: resp.response_id,
                label,
                note: Some(format!("格口 {code} 不在對照表")),
                reason: Some("CHUTE_UNKNOWN".into()),
            },
        },
        None => Decision {
            code: default_code.into(),
            cid: default_cid,
            source: ChuteSource::Default,
            response_id: resp.response_id,
            label,
            note: resp.error_code.clone().map(|e| format!("{e}: {}", resp.message.clone().unwrap_or_default())).or(Some("中介機未給格口".into())),
            reason: error_code.or(Some("NO_CHANNEL".into())),
        },
    }
}

impl ChuteResolver {
    pub fn spawn(
        app: AppState,
        mw: Middleware,
        tracker: TrackerHandle,
        chutes: HashMap<String, ChuteRow>,
        mut rx: mpsc::Receiver<ChuteRequest>,
        cancel: CancellationToken,
    ) -> Self {
        let me = Self { app, mw, tracker, chutes: Arc::new(RwLock::new(chutes)), latency: Arc::new(RwLock::new(LatencyWindow::new(2000))) };
        let this = me.clone();
        tokio::spawn(async move {
            loop {
                let req = tokio::select! {
                    _ = cancel.cancelled() => break,
                    r = rx.recv() => match r { Some(r) => r, None => break },
                };
                let r = this.clone();
                tokio::spawn(async move { r.resolve(req).await });
            }
        });
        me
    }

    pub fn set_chutes(&self, chutes: HashMap<String, ChuteRow>) {
        *self.chutes.write().unwrap() = chutes;
    }

    /// 最近一小時的查詢延遲；預算 = `~P→~O` 實測中位 1.3s 扣掉相機 0.2s ≈ 1100ms
    pub fn latency_stats(&self) -> LatencyStats {
        self.latency.read().unwrap().stats(crate::db::now_ms(), 3_600_000, 1100)
    }

    async fn resolve(&self, req: ChuteRequest) {
        let ChuteRequest { key, ulid, barcode, notify_only } = req;
        let started = std::time::Instant::now();
        let general = self.app.config.current().general;
        let default_code = general.default_chute;
        // 幾秒內同碼剛正常分過：不問中介機（問了它會再記一次完成），直接走異常口讓人核對面單
        let held = if notify_only { None } else { check_reentry_hold(&self.app.db, &barcode, &ulid, general.reentry_hold_ms).await };
        if let Some((prev_ms, now)) = held {
            let decision = self.fallback(&default_code, format!("{:.1} 秒前同條碼剛分過，疑似讀到鄰件條碼", (now - prev_ms) as f64 / 1000.0), "REENTRY");
            event_log::log(&self.app.db, Level::Warn, "chute", "reentry_hold", format!("條碼 {barcode} → {}：{}", decision.code, decision.note.clone().unwrap_or_default()));
            event_bus::emit("chute-resolved", serde_json::json!({ "key": key, "barcode": barcode, "chute": decision.code, "source": decision.source, "elapsed_ms": started.elapsed().as_millis() as i64, "note": decision.note }));
            event_bus::emit("parcel-alert", serde_json::json!({ "kind": "reentry_hold", "barcode": barcode, "message": format!("條碼 {barcode} 幾秒前剛分過又進線，已送異常口，請核對面單") }));
            self.tracker.chute_result(key, decision.code, decision.cid, decision.source, decision.response_id, decision.reason, None);
            return;
        }
        let result = self.mw.query_parcel(&barcode).await;
        let elapsed = started.elapsed().as_millis() as i64;
        if notify_only {
            // NoRead：中介機拍照存證與計數；本機已走預設口，回什麼都不改
            match result {
                Ok(_) => tracing::info!(%barcode, elapsed, "已通知中介機讀碼失敗"),
                Err(e) => event_log::log(&self.app.db, Level::Warn, "chute", "noread_notify", format!("通知中介機讀碼失敗未成功（{elapsed}ms）：{e}")),
            }
            return;
        }
        let mut decision = match &result {
            Ok(resp) => {
                let chutes = self.chutes.read().unwrap();
                decide(resp, &chutes, &default_code)
            }
            Err(e) => self.fallback(&default_code, e.to_string(), "MW_UNREACHABLE"),
        };
        // 面單先抓下來才算決定；抓不到就改走預設口、不回報、不印（對齊舊版）。
        // 目標格口沒接印表機（L5／R5／直通口）就不需要面單，不抓、也不因圖片服務故障被拖去異常口
        let mut payload = None;
        let has_printer = self.chutes.read().unwrap().get(&decision.code).is_some_and(|r| r.printer_port.is_some());
        if !has_printer {
            decision.label = None;
        }
        if let Some(info) = decision.label.take() {
            match self.fetch(&info.label_path).await {
                Ok(bytes) => payload = Some(LabelPayload { bytes, print_profile: info.print_profile, is_error_label: info.is_error_label }),
                Err(e) => decision = self.fallback(&default_code, format!("面單下載失敗（{}ms）：{e}", started.elapsed().as_millis()), "LABEL_FETCH_FAILED"),
            }
        }
        let elapsed = started.elapsed().as_millis() as i64;
        self.latency.write().unwrap().push(crate::db::now_ms(), elapsed.clamp(0, u32::MAX as i64) as u32);
        if let Some(note) = &decision.note {
            event_log::log(&self.app.db, Level::Warn, "chute", "resolve", format!("條碼 {barcode} → {}（{elapsed}ms）：{note}", decision.code));
        } else {
            tracing::info!(%barcode, chute = %decision.code, elapsed, label = payload.is_some(), "格口");
        }
        event_bus::emit("chute-resolved", serde_json::json!({ "key": key, "barcode": barcode, "chute": decision.code, "source": decision.source, "elapsed_ms": elapsed, "note": decision.note }));
        self.tracker.chute_result(key, decision.code, decision.cid, decision.source, decision.response_id, decision.reason.clone(), payload);
        // 決定先送出去，前科查詢另外跑：它要查 DB、可能還要通知中介機，不能拖住分揀
        let this = self.clone();
        let (source, reason) = (decision.source, decision.reason);
        tokio::spawn(async move { this.history_alert(&ulid, &barcode, source, reason.as_deref()).await });
    }

    /// 同一條碼今天的前科：反覆因同一原因走異常口（門市關轉件被一投再投，今天有一件投了 4 次），
    /// 或已完成卻又進線（格口滿溢彈回皮帶／人工重投）。只看今天、排除這一件自己；
    /// 查不到（DB 忙）就當沒有——這是提示，不能影響分揀。
    async fn history_alert(&self, ulid: &str, barcode: &str, source: ChuteSource, reason: Option<&str>) {
        let prior = load_prior(&self.app.db, barcode, ulid, crate::db::today_start_ms()).await;
        let Some(alert) = history_verdict(&prior, source, reason) else { return };
        match alert {
            HistoryAlert::RepeatDefault { count, reason } => {
                let msg = format!("條碼 {barcode} 今天第 {count} 次因「{}」走異常口，請下架不要再投", reason_label(&reason));
                event_log::log(&self.app.db, Level::Warn, "chute", "repeat_default", msg.clone());
                event_bus::emit("parcel-alert", serde_json::json!({ "kind": "repeat_default", "barcode": barcode, "count": count, "reason": reason, "message": msg }));
                // 中介機端會出聲（自訂類別走通用語音）＋ toast，現場才聽得到
                self.mw.device_alert("REPEAT_DEFAULT", &msg).await;
            }
            HistoryAlert::ReEntry { prev_chute, prev_started_ms } => {
                let at = chrono::DateTime::from_timestamp_millis(prev_started_ms).map(|t| t.with_timezone(&chrono::Local).format("%H:%M:%S").to_string()).unwrap_or_default();
                let msg = format!("條碼 {barcode} 已於 {at} 落 {prev_chute} 完成，卻再次進線；請檢查該格口是否已滿或包裹被退回", );
                event_log::log(&self.app.db, Level::Warn, "chute", "re_entry", msg.clone());
                event_bus::emit("parcel-alert", serde_json::json!({ "kind": "re_entry", "barcode": barcode, "prev_chute": prev_chute, "message": msg }));
            }
        }
    }

    /// 走預設口、不回報、不印
    fn fallback(&self, default_code: &str, note: String, reason: &str) -> Decision {
        let chutes = self.chutes.read().unwrap();
        let cid = chutes.get(default_code).map(|r| r.cid).unwrap_or(Cid(1007301));
        Decision { code: default_code.to_string(), cid, source: ChuteSource::Default, response_id: None, label: None, note: Some(note), reason: Some(reason.into()) }
    }

    /// 面單來源：`http(s)://` 向中介機抓，其餘當本機路徑（同一台機器上的 `direct_print` 模式）
    async fn fetch(&self, path: &str) -> anyhow::Result<Vec<u8>> {
        if path.starts_with("http://") || path.starts_with("https://") {
            Ok(self.mw.fetch_label(path).await?)
        } else {
            Ok(tokio::fs::read(path).await.map_err(|e| anyhow::anyhow!("讀取 {path} 失敗: {e}"))?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 延遲統計_只看窗口內_超預算比例() {
        let mut w = LatencyWindow::new(3);
        w.push(0, 100);
        w.push(1_000, 300);
        w.push(2_000, 1_500);
        w.push(3_000, 400); // 超過容量，最早的 100 被擠掉
        let s = w.stats(3_000, 10_000, 1100);
        assert_eq!((s.samples, s.p50_ms, s.max_ms), (3, 400, 1_500));
        assert!((s.over_budget_pct - 33.3).abs() < 0.5);
        let s = w.stats(3_000, 500, 1100);
        assert_eq!(s.samples, 1, "窗口外的不算");
        assert_eq!(LatencyWindow::new(3).stats(0, 1, 1100).samples, 0);
    }

    fn chutes() -> HashMap<String, ChuteRow> {
        let mut m = HashMap::new();
        for (code, cid, enabled) in [("L1", 1000323, true), ("R5", 1006324, false), ("RS", 1007301, true), ("LS", 1007323, true)] {
            m.insert(code.to_string(), ChuteRow { code: code.into(), cid: Cid(cid), printer_port: None, enabled, label: code.into(), sort_order: 0 });
        }
        m
    }

    #[test]
    fn 正常回應對到格口() {
        let resp = ParcelResp { channel_code: Some("L1".into()), response_id: Some(7), label_path: Some("http://x/a.png".into()), print_profile: Some("PAPER-01#100*150".into()), ..Default::default() };
        let d = decide(&resp, &chutes(), "RS");
        assert_eq!((d.code.as_str(), d.cid, d.source, d.response_id), ("L1", Cid(1000323), ChuteSource::Api, Some(7)));
        assert!(d.label.is_some());
    }

    #[test]
    fn 直通件_ls_也是合法格口() {
        let resp = ParcelResp { channel_code: Some("LS".into()), response_id: Some(8), ..Default::default() };
        let d = decide(&resp, &chutes(), "RS");
        assert_eq!((d.code.as_str(), d.source), ("LS", ChuteSource::Api));
        assert!(d.label.is_none());
    }

    #[test]
    fn noread_走預設口不回報() {
        let resp = ParcelResp { error_code: Some("NOREAD".into()), ..Default::default() };
        let d = decide(&resp, &chutes(), "RS");
        assert_eq!((d.code.as_str(), d.source, d.response_id), ("RS", ChuteSource::NoRead, None));
    }

    #[test]
    fn 業務錯誤無格口_走預設口() {
        let resp = ParcelResp { error_code: Some("STORE_CLOSED".into()), message: Some("門市關轉".into()), ..Default::default() };
        let d = decide(&resp, &chutes(), "RS");
        assert_eq!((d.code.as_str(), d.source), ("RS", ChuteSource::Default));
        assert!(d.note.unwrap().contains("STORE_CLOSED"));
        assert_eq!(d.reason.as_deref(), Some("STORE_CLOSED"), "原因代碼要跟中介機的錯誤碼一樣，兩邊統計才對得起來");
    }

    #[test]
    fn 錯誤提示面單_照正常流程() {
        let resp = ParcelResp { channel_code: Some("L1".into()), response_id: Some(-3), label_path: Some("http://x/@error/a.png".into()), is_error_label: true, error_code: Some("NOT_FOUND".into()), ..Default::default() };
        let d = decide(&resp, &chutes(), "RS");
        assert_eq!((d.code.as_str(), d.source, d.response_id), ("L1", ChuteSource::Api, Some(-3)));
        assert!(d.label.unwrap().is_error_label);
        assert_eq!(d.reason.as_deref(), Some("NOT_FOUND"), "錯誤提示面單雖然照分，原因也要記");
    }

    #[test]
    fn 停用或不存在的格口_走預設口但保留回報() {
        let resp = ParcelResp { channel_code: Some("R5".into()), response_id: Some(9), ..Default::default() };
        let d = decide(&resp, &chutes(), "RS");
        assert_eq!((d.code.as_str(), d.source, d.response_id), ("RS", ChuteSource::Default, Some(9)));
        let resp = ParcelResp { channel_code: Some("C03".into()), response_id: Some(9), ..Default::default() };
        let d = decide(&resp, &chutes(), "RS");
        assert_eq!(d.code, "RS");
        assert!(d.note.unwrap().contains("不在對照表"));
        assert_eq!(d.reason.as_deref(), Some("CHUTE_UNKNOWN"));
    }
    #[test]
    fn 再進線攔截_幾秒內正常分過就攔_異常口與關閉不攔() {
        let in_flight = PriorParcel { status: 2, chute_code: Some("L2".into()), chute_source: Some("api".into()), chute_reason: None, started_ms: 1000 };
        assert_eq!(reentry_hold(&[in_flight.clone()], 8000), Some(1000), "還在路上的件也算剛分過");
        assert_eq!(reentry_hold(&[in_flight.clone()], 0), None, "0 = 關閉");
        let default = PriorParcel { status: 3, chute_code: Some("RS".into()), chute_source: Some("default".into()), chute_reason: Some("STORE_CLOSED".into()), started_ms: 1000 };
        assert_eq!(reentry_hold(&[default], 8000), None, "上次走異常口的不攔——那是重投迴圈");
        assert_eq!(reentry_hold(&[], 8000), None);
    }

    #[tokio::test]
    async fn 再進線攔截_查資料庫_只攔窗口內正常分過的同碼_不含自己() {
        let dir = std::env::temp_dir().join(format!("reentry-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::init(&dir).await.unwrap();
        let now = crate::db::now_ms();
        let seed = |bc: &'static str, chute: &'static str, source: &'static str, reason: Option<&'static str>, status: i64, ago_ms: i64| {
            let db = db.clone();
            async move {
                let u = ulid::Ulid::generate().to_string();
                sqlx::query("INSERT INTO parcels (ulid, barcode, chute_code, chute_source, chute_reason, status, started_at, started_ms, updated_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
                    .bind(&u).bind(bc).bind(chute).bind(source).bind(reason).bind(status)
                    .bind(crate::db::local_ts(now - ago_ms)).bind(now - ago_ms).bind(now - ago_ms)
                    .execute(&db).await.unwrap();
                u
            }
        };
        // A：3 秒前中介機給了 L2、還在路上（狀態 2）→ 攔
        seed("A1234567890", "L2", "api", None, 2, 3000).await;
        let hit = check_reentry_hold(&db, "A1234567890", "me", 6000).await;
        assert_eq!(hit.map(|(prev, _)| now - prev >= 3000), Some(true));
        // 同一件自己（ulid 相同）不算前科
        let me = seed("B1234567890", "L1", "api", None, 3, 1000).await;
        assert!(check_reentry_hold(&db, "B1234567890", &me, 6000).await.is_none());
        // C：10 秒前分過 → 超出窗口不攔
        seed("C1234567890", "L1", "api", None, 3, 10_000).await;
        assert!(check_reentry_hold(&db, "C1234567890", "me", 6000).await.is_none());
        // D：2 秒前走異常口（門市關轉）→ 重投迴圈，不攔
        seed("D1234567890", "RS", "default", Some("STORE_CLOSED"), 3, 2000).await;
        assert!(check_reentry_hold(&db, "D1234567890", "me", 6000).await.is_none());
        // 關閉
        assert!(check_reentry_hold(&db, "A1234567890", "me", 0).await.is_none());
    }

    fn prior(status: i64, chute: &str, reason: Option<&str>, started_ms: i64) -> PriorParcel {
        // 有原因代碼的都是走異常口（default），沒有的是中介機正常給格口（api）
        let source = if reason.is_some() { "default" } else { "api" };
        PriorParcel { status, chute_code: Some(chute.into()), chute_source: Some(source.into()), chute_reason: reason.map(String::from), started_ms }
    }

    #[test]
    fn 同原因再走異常口_提示第幾次() {
        let p = vec![prior(3, "RS", Some("STORE_CLOSED"), 1000), prior(3, "RS", Some("STORE_CLOSED"), 500)];
        assert_eq!(
            history_verdict(&p, ChuteSource::Default, Some("STORE_CLOSED")),
            Some(HistoryAlert::RepeatDefault { count: 3, reason: "STORE_CLOSED".into() })
        );
        // 原因不同不算重複（上次逾時、這次關轉）；上次走異常口雖然狀態也是「完成」，那是重投的常態，不算「已完成又進線」
        assert_eq!(
            history_verdict(&[prior(3, "RS", Some("TIMEOUT"), 1000)], ChuteSource::Default, Some("STORE_CLOSED")),
            None
        );
        assert_eq!(history_verdict(&[prior(3, "RS", Some("NOREAD"), 1000)], ChuteSource::Api, None), None);
    }

    #[test]
    fn 已完成又進線_提示上次落哪() {
        let p = vec![prior(4, "L2", None, 2000), prior(3, "L5", None, 1000)];
        assert_eq!(
            history_verdict(&p, ChuteSource::Api, None),
            Some(HistoryAlert::ReEntry { prev_chute: "L5".into(), prev_started_ms: 1000 })
        );
        // 上次遺失、沒完成過 → 不提示
        assert_eq!(history_verdict(&[prior(4, "L2", None, 2000)], ChuteSource::Api, None), None);
        // 沒前科
        assert_eq!(history_verdict(&[], ChuteSource::Api, None), None);
    }

    #[test]
    fn 完成過的包裹再進線後走異常口_算重複異常不算格口滿() {
        let p = vec![prior(3, "RS", Some("STORE_CLOSED"), 1000), prior(3, "L1", None, 500)];
        assert!(matches!(history_verdict(&p, ChuteSource::Default, Some("STORE_CLOSED")), Some(HistoryAlert::RepeatDefault { count: 2, .. })));
    }

    #[tokio::test]
    async fn 前科查詢_只看同條碼_排除自己_只看起點之後() {
        let dir = std::env::temp_dir().join(format!("chute-prior-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::init(&dir).await.unwrap();
        let ins = |ulid: &str, barcode: &str, source: &str, reason: Option<&str>, status: i64, ms: i64| {
            let (ulid, barcode, source, reason) = (ulid.to_string(), barcode.to_string(), source.to_string(), reason.map(String::from));
            let db = db.clone();
            async move {
                sqlx::query("INSERT INTO parcels (ulid, barcode, chute_code, chute_source, chute_reason, status, started_at, started_ms, updated_ms) VALUES (?, ?, 'RS', ?, ?, ?, '2026-09-18 19:00:00.000', ?, ?)")
                    .bind(ulid).bind(barcode).bind(source).bind(reason).bind(status).bind(ms).bind(ms)
                    .execute(&db).await.unwrap();
            }
        };
        ins("A", "X1", "default", Some("STORE_CLOSED"), 3, 1_000).await;
        ins("B", "X1", "default", Some("STORE_CLOSED"), 3, 2_000).await;
        ins("C", "X1", "default", Some("STORE_CLOSED"), 1, 3_000).await; // 這一件自己
        ins("D", "X2", "api", None, 3, 2_500).await; // 別的條碼
        ins("E", "X1", "default", Some("STORE_CLOSED"), 3, 500).await; // 起點之前（昨天）
        let prior = load_prior(&db, "X1", "C", 1_000).await;
        assert_eq!(prior.iter().map(|p| p.started_ms).collect::<Vec<_>>(), vec![2_000, 1_000], "新的在前、排除自己與昨天");
        assert!(matches!(history_verdict(&prior, ChuteSource::Default, Some("STORE_CLOSED")), Some(HistoryAlert::RepeatDefault { count: 3, .. })));
    }


}
