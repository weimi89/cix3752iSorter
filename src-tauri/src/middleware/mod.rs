//! 中介機（cix3752iLabelPrint）client。契約：`cix3752iLabelPrint/docs/local-http-api.md`、`device-alert-api.md`。
//!
//! - `GET /api/parcel/{code}`：取格口、列印 profile、面單路徑、`response_id`
//! - `POST /api/report`：回報（帶 `response_id`），走本地佇列＋指數退避，重啟不丟
//! - `POST /api/device-alert`：卡件／印表機異常廣播，fire-and-forget，同型別節流

pub mod report_queue;

use std::time::Duration;

use serde::Deserialize;
use tokio::sync::watch;

use crate::config::AppConfig;

/// 面單圖大小上限：現場面單 PNG 幾百 KB，超過這個一定不是面單，不讓它塞爆狀態機佇列
const LABEL_MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, Default, Deserialize, serde::Serialize, PartialEq)]
pub struct ParcelResp {
    pub channel_code: Option<String>,
    pub print_profile: Option<String>,
    /// 缺欄位 = `direct_print` 模式或下載失敗，不是 null
    #[serde(default)]
    pub label_path: Option<String>,
    pub response_id: Option<i64>,
    #[serde(default)]
    pub is_error_label: bool,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Deserialize)]
struct Envelope {
    data: ParcelResp,
}

#[derive(Clone)]
pub struct Middleware {
    http: reqwest::Client,
    cfg: watch::Receiver<AppConfig>,
}

#[derive(Debug, thiserror::Error)]
pub enum MwError {
    #[error("逾時（{0}ms）")]
    Timeout(u64),
    #[error("連線失敗: {0}")]
    Connect(String),
    #[error("HTTP {0}: {1}")]
    Status(u16, String),
    #[error("回應解析失敗: {0}")]
    Parse(String),
}

impl MwError {
    /// 4xx 類：重試也不會變，佇列直接標失敗
    pub fn is_permanent(&self) -> bool {
        matches!(self, MwError::Status(s, _) if (400..500).contains(s))
    }
}

impl Middleware {
    pub fn new(cfg: watch::Receiver<AppConfig>) -> Self {
        let http = reqwest::Client::builder()
            .pool_max_idle_per_host(8)
            .tcp_keepalive(Duration::from_secs(30))
            .connect_timeout(Duration::from_millis(800))
            .build()
            .expect("reqwest client");
        Self { http, cfg }
    }

    fn url(&self, path: &str) -> String {
        let base = self.cfg.borrow().middleware.base_url.clone();
        format!("{}/{}", base.trim_end_matches('/'), path.trim_start_matches('/'))
    }

    /// 查格口。呼叫端自行決定逾時後怎麼辦（狀態機在 `~O` 時會用預設口）。
    pub async fn query_parcel(&self, code: &str) -> Result<ParcelResp, MwError> {
        let timeout_ms = self.cfg.borrow().middleware.parcel_timeout_ms;
        let url = self.url(&format!("api/parcel/{}", urlencoding(code)));
        let fut = self.http.get(&url).timeout(Duration::from_millis(timeout_ms)).send();
        let resp = match fut.await {
            Ok(r) => r,
            Err(e) if e.is_timeout() => return Err(MwError::Timeout(timeout_ms)),
            Err(e) => return Err(MwError::Connect(e.to_string())),
        };
        let status = resp.status();
        let body = resp.text().await.map_err(|e| MwError::Parse(e.to_string()))?;
        if !status.is_success() {
            return Err(MwError::Status(status.as_u16(), body.chars().take(200).collect()));
        }
        serde_json::from_str::<Envelope>(&body).map(|e| e.data).map_err(|e| MwError::Parse(format!("{e}: {}", body.chars().take(200).collect::<String>())))
    }

    /// 下載面單（`http` 模式的 `label_path`）
    pub async fn fetch_label(&self, url: &str) -> Result<Vec<u8>, MwError> {
        let timeout_ms = self.cfg.borrow().middleware.label_timeout_ms;
        let resp = match self.http.get(url).timeout(Duration::from_millis(timeout_ms)).send().await {
            Ok(r) => r,
            Err(e) if e.is_timeout() => return Err(MwError::Timeout(timeout_ms)),
            Err(e) => return Err(MwError::Connect(e.to_string())),
        };
        let status = resp.status();
        if !status.is_success() {
            return Err(MwError::Status(status.as_u16(), String::new()));
        }
        if resp.content_length().is_some_and(|n| n > LABEL_MAX_BYTES) {
            return Err(MwError::Parse(format!("面單超過 {} MB", LABEL_MAX_BYTES / 1_048_576)));
        }
        let bytes = resp.bytes().await.map_err(|e| MwError::Parse(e.to_string()))?;
        if bytes.len() as u64 > LABEL_MAX_BYTES {
            return Err(MwError::Parse(format!("面單超過 {} MB", LABEL_MAX_BYTES / 1_048_576)));
        }
        Ok(bytes.to_vec())
    }

    pub async fn report(&self, response_id: i64) -> Result<(), MwError> {
        let timeout_ms = self.cfg.borrow().middleware.report_timeout_ms;
        let url = self.url("api/report");
        let resp = match self
            .http
            .post(&url)
            .json(&serde_json::json!({ "response_id": response_id }))
            .timeout(Duration::from_millis(timeout_ms))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) if e.is_timeout() => return Err(MwError::Timeout(timeout_ms)),
            Err(e) => return Err(MwError::Connect(e.to_string())),
        };
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(MwError::Status(status.as_u16(), body.chars().take(200).collect()))
        }
    }

    /// 設備異常廣播；失敗只記錄，不影響分揀
    pub async fn device_alert(&self, kind: &str, message: &str) {
        let timeout_ms = self.cfg.borrow().middleware.alert_timeout_ms;
        let url = self.url("api/device-alert");
        let r = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "type": kind, "message": message }))
            .timeout(Duration::from_millis(timeout_ms))
            .send()
            .await;
        match r {
            Ok(resp) if resp.status().is_success() => tracing::info!(kind, message, "設備異常已通知中介機"),
            Ok(resp) => tracing::warn!(kind, message, status = %resp.status(), "設備異常通知被拒"),
            Err(e) => tracing::warn!(kind, message, "設備異常通知失敗: {e}"),
        }
    }
}

fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 回應解析_正常與缺欄位() {
        let ok: Envelope = serde_json::from_str(r#"{"data":{"channel_code":"L1","print_profile":"PAPER-01#100*150","label_path":"http://x/images/a.png","response_id":123}}"#).unwrap();
        assert_eq!(ok.data.channel_code.as_deref(), Some("L1"));
        assert_eq!(ok.data.response_id, Some(123));
        assert!(!ok.data.is_error_label);

        let noread: Envelope = serde_json::from_str(r#"{"data":{"channel_code":null,"print_profile":null,"response_id":null,"error_code":"NOREAD","message":"讀碼失敗"}}"#).unwrap();
        assert_eq!(noread.data.error_code.as_deref(), Some("NOREAD"));
        assert!(noread.data.label_path.is_none());

        let err: Envelope = serde_json::from_str(r#"{"data":{"channel_code":"C03","print_profile":"100x100","label_path":"http://x/@error/a.png","response_id":-1,"is_error_label":true,"error_code":"STORE_CLOSED","message":"x"}}"#).unwrap();
        assert!(err.data.is_error_label);
        assert_eq!(err.data.response_id, Some(-1));
    }

    #[test]
    fn 條碼網址編碼() {
        assert_eq!(urlencoding("SF1234567890123"), "SF1234567890123");
        assert_eq!(urlencoding("a b/c"), "a%20b%2Fc");
    }
}
