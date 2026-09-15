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
}

pub struct Decision {
    pub code: String,
    pub cid: Cid,
    pub source: ChuteSource,
    pub response_id: Option<i64>,
    pub label: Option<LabelInfo>,
    pub note: Option<String>,
}

/// 純函式：把中介機回應對到格口表（可單元測試）
pub fn decide(resp: &ParcelResp, chutes: &HashMap<String, ChuteRow>, default_code: &str) -> Decision {
    let default_cid = chutes.get(default_code).map(|r| r.cid).unwrap_or(Cid(1007301));
    let label = resp.label_path.as_ref().map(|p| LabelInfo {
        label_path: p.clone(),
        print_profile: resp.print_profile.clone(),
        is_error_label: resp.is_error_label,
    });

    if resp.error_code.as_deref().is_some_and(|c| c.eq_ignore_ascii_case("NOREAD")) {
        return Decision { code: default_code.into(), cid: default_cid, source: ChuteSource::NoRead, response_id: None, label: None, note: None };
    }
    match resp.channel_code.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(code) => match chutes.get(code) {
            Some(row) if row.enabled => Decision {
                code: row.code.clone(),
                cid: row.cid,
                source: ChuteSource::Api,
                response_id: resp.response_id,
                label,
                note: resp.error_code.clone().map(|e| format!("{e}: {}", resp.message.clone().unwrap_or_default())),
            },
            Some(_) => Decision {
                code: default_code.into(),
                cid: default_cid,
                source: ChuteSource::Default,
                response_id: resp.response_id,
                label,
                note: Some(format!("格口 {code} 已停用")),
            },
            None => Decision {
                code: default_code.into(),
                cid: default_cid,
                source: ChuteSource::Default,
                response_id: resp.response_id,
                label,
                note: Some(format!("格口 {code} 不在對照表")),
            },
        },
        None => Decision {
            code: default_code.into(),
            cid: default_cid,
            source: ChuteSource::Default,
            response_id: resp.response_id,
            label,
            note: resp.error_code.clone().map(|e| format!("{e}: {}", resp.message.clone().unwrap_or_default())).or(Some("中介機未給格口".into())),
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
        let me = Self { app, mw, tracker, chutes: Arc::new(RwLock::new(chutes)) };
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

    async fn resolve(&self, req: ChuteRequest) {
        let ChuteRequest { key, ulid: _, barcode, notify_only } = req;
        let started = std::time::Instant::now();
        let default_code = self.app.config.current().general.default_chute;
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
            Err(e) => self.fallback(&default_code, e.to_string()),
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
                Err(e) => decision = self.fallback(&default_code, format!("面單下載失敗（{}ms）：{e}", started.elapsed().as_millis())),
            }
        }
        let elapsed = started.elapsed().as_millis() as i64;
        if let Some(note) = &decision.note {
            event_log::log(&self.app.db, Level::Warn, "chute", "resolve", format!("條碼 {barcode} → {}（{elapsed}ms）：{note}", decision.code));
        } else {
            tracing::info!(%barcode, chute = %decision.code, elapsed, label = payload.is_some(), "格口");
        }
        event_bus::emit("chute-resolved", serde_json::json!({ "key": key, "barcode": barcode, "chute": decision.code, "source": decision.source, "elapsed_ms": elapsed, "note": decision.note }));
        self.tracker.chute_result(key, decision.code, decision.cid, decision.source, decision.response_id, payload);
    }

    /// 走預設口、不回報、不印
    fn fallback(&self, default_code: &str, note: String) -> Decision {
        let chutes = self.chutes.read().unwrap();
        let cid = chutes.get(default_code).map(|r| r.cid).unwrap_or(Cid(1007301));
        Decision { code: default_code.to_string(), cid, source: ChuteSource::Default, response_id: None, label: None, note: Some(note) }
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

    fn chutes() -> HashMap<String, ChuteRow> {
        let mut m = HashMap::new();
        for (code, cid, enabled) in [("L1", 1000323, true), ("R5", 1006324, false), ("RS", 1007301, true), ("LS", 1007323, true)] {
            m.insert(code.to_string(), ChuteRow { code: code.into(), cid: Cid(cid), printer_port: None, enabled });
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
    }

    #[test]
    fn 錯誤提示面單_照正常流程() {
        let resp = ParcelResp { channel_code: Some("L1".into()), response_id: Some(-3), label_path: Some("http://x/@error/a.png".into()), is_error_label: true, error_code: Some("NOT_FOUND".into()), ..Default::default() };
        let d = decide(&resp, &chutes(), "RS");
        assert_eq!((d.code.as_str(), d.source, d.response_id), ("L1", ChuteSource::Api, Some(-3)));
        assert!(d.label.unwrap().is_error_label);
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
    }
}
