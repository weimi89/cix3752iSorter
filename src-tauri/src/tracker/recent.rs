//! 最近上線的包裹（記憶體）：查格口時的「同碼幾秒內剛分過」查這裡，不查資料庫。
//!
//! 查格口有時限，不能等磁碟：工控機硬碟偶爾卡住數秒到數十秒，查資料庫會跟著卡，包裹來不及分格口就走異常口；
//! 而且那時寫入端也落後，資料庫裡還沒有最近幾秒的結果，查了也查不到。
//! 內容由寫入端在收下每筆包裹寫入時同步更新（`Store::send`），欄位值與寫進 parcels 的相同，只是不必等寫完。

use std::collections::HashMap;
use std::sync::Mutex;

use crate::chute::PriorParcel;
use crate::config::REENTRY_HOLD_MAX_MS;

/// 與 `chute::load_prior` 的 `LIMIT` 相同
const PRIOR_LIMIT: usize = 10;

struct Entry {
    barcode: String,
    prior: PriorParcel,
}

/// 以 ulid 為鍵，只留上線時間在 `REENTRY_HOLD_MAX_MS` 內的件（約幾十到一百多件）
#[derive(Default)]
pub struct RecentParcels {
    inner: Mutex<HashMap<String, Entry>>,
}

impl RecentParcels {
    pub fn new() -> Self {
        Self::default()
    }

    /// 記下（或覆蓋）一件包裹目前的狀態；上線超過保留窗口的件順便清掉
    pub fn record(&self, ulid: &str, barcode: &str, prior: PriorParcel, now_ms: i64) {
        let cutoff = now_ms - REENTRY_HOLD_MAX_MS;
        let mut m = self.inner.lock().unwrap();
        m.retain(|_, e| e.prior.started_ms >= cutoff);
        if prior.started_ms >= cutoff {
            m.insert(ulid.to_string(), Entry { barcode: barcode.to_string(), prior });
        }
    }

    /// 同一條碼從 `since_ms` 起上線的其他包裹（排除自己），新的在前——與 `chute::load_prior` 的查詢條件相同。
    /// `since_ms` 早於保留窗口時只回得到窗口內的件
    pub fn prior(&self, barcode: &str, ulid: &str, since_ms: i64) -> Vec<PriorParcel> {
        let m = self.inner.lock().unwrap();
        let mut rows: Vec<PriorParcel> = m
            .iter()
            .filter(|(u, e)| e.barcode == barcode && u.as_str() != ulid && e.prior.started_ms >= since_ms)
            .map(|(_, e)| e.prior.clone())
            .collect();
        rows.sort_by_key(|p| std::cmp::Reverse(p.started_ms));
        rows.truncate(PRIOR_LIMIT);
        rows
    }

    #[cfg(test)]
    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(chute: &str, source: &str, started_ms: i64) -> PriorParcel {
        PriorParcel { status: 2, chute_code: Some(chute.into()), chute_source: Some(source.into()), chute_reason: None, started_ms }
    }

    #[test]
    fn 只回同碼_排除自己_窗口內_新的在前() {
        let r = RecentParcels::new();
        let now = 1_000_000;
        r.record("u1", "A1", p("L1", "api", now - 5_000), now);
        r.record("u2", "A1", p("L2", "api", now - 2_000), now);
        r.record("u3", "B1", p("L3", "api", now - 1_000), now);
        r.record("me", "A1", p("L4", "api", now - 500), now);
        let got = r.prior("A1", "me", now - 6_000);
        assert_eq!(got.iter().map(|x| x.chute_code.clone().unwrap()).collect::<Vec<_>>(), vec!["L2", "L1"]);
        assert_eq!(r.prior("A1", "me", now - 3_000).len(), 1, "早於 since 的不算");
    }

    #[test]
    fn 同一件更新時覆蓋_不重複() {
        let r = RecentParcels::new();
        let now = 1_000_000;
        r.record("u1", "A1", p("", "pending", now - 1_000), now);
        r.record("u1", "A1", p("L1", "api", now - 1_000), now);
        let got = r.prior("A1", "me", now - 6_000);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].chute_source.as_deref(), Some("api"));
    }

    #[test]
    fn 超過保留窗口的件會被清掉_記憶體不會一直長() {
        let r = RecentParcels::new();
        let now = 10_000_000;
        r.record("old", "A1", p("L1", "api", now - REENTRY_HOLD_MAX_MS - 1), now);
        assert_eq!(r.count(), 0, "一進來就已超過窗口的不留");
        r.record("u1", "A1", p("L1", "api", now - 1_000), now);
        r.record("u2", "B1", p("L1", "api", now + REENTRY_HOLD_MAX_MS), now + REENTRY_HOLD_MAX_MS);
        assert_eq!(r.count(), 1, "下一筆寫入時，已超過窗口的 u1 被清掉");
    }
}
