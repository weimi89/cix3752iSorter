//! 班次報表：把一天的統計套上門檻，挑出「今天最該處理的幾件事」，用現場聽得懂的話寫。
//! 主管不會盯著看板，一天看一次這頁就要知道要改什麼；門檻寫死在這裡，調整時看 9/21 的數字：
//! 讀碼失敗 3.6%、L2 完成後又進線 4%、第四模組卡件佔三分之一、讀到鄰件條碼 17 件。

use serde::Serialize;

use super::stats::Overview;

#[derive(Serialize, Debug, PartialEq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warning,
    Info,
}

#[derive(Serialize, Debug, Clone)]
pub struct Finding {
    pub level: Level,
    /// 給前端連結與圖示用的代碼
    pub code: &'static str,
    /// 一句話講問題與數字
    pub title: String,
    /// 補充數據
    pub detail: String,
    /// 現場可以做什麼
    pub action: String,
}

fn pct(n: i64, total: i64) -> f64 {
    if total <= 0 { 0.0 } else { (n as f64 * 1000.0 / total as f64).round() / 10.0 }
}

/// 依門檻挑問題；順序＝嚴重度（紅在前）。沒件的日子回空
pub fn findings(o: &Overview) -> Vec<Finding> {
    let total = o.range.total;
    if total == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();

    // 讀碼失敗率：3% 黃、5% 紅（9/18 一晚從 1.9% 爬到 3.6% 沒人發現）。原因是系統從讀碼站回應判的
    let noread = o.range.noread;
    let noread_pct = pct(noread, total);
    if noread_pct >= 3.0 {
        let causes_total: i64 = o.noread_causes.iter().map(|c| c.count).sum();
        let list = o.noread_causes.iter().map(|c| format!("{} {} 件", cause_label(&c.key), c.count)).collect::<Vec<_>>().join("、");
        let top = o.noread_causes.first();
        let mut detail = if causes_total > 0 { list } else { "沒有讀碼站回應可判原因".to_string() };
        // 頂部「讀碼失敗」大字只算落到異常口的；讀不到碼後又被分揀機弄掉的算分揀機異常，這裡把差額講清楚
        let lost_after = noread - o.range.noread_landed;
        if lost_after > 0 {
            detail.push_str(&format!("；其中 {lost_after} 件之後被分揀機弄掉，算在分揀機異常"));
        }
        let action = match top.map(|c| c.key.as_str()) {
            Some("no_code") => "讀碼器有拍但讀不到：投料時面單朝上、放皮帶中央；亮面袋反光就調讀碼站曝光",
            Some("neighbor") => "讀碼器的讀碼區域（ROI）排除畫面上緣進料區；投料時前後件別靠太近",
            Some("no_frame") => "讀碼器沒回應：檢查觸發光電與讀碼器連線",
            Some("bad_code") => "讀到的是包材條碼或內部序號：面單貼在最大面、別被其他條碼蓋到",
            _ => "翻「異常存證」的照片看是投料還是讀碼站的問題",
        };
        out.push(Finding { level: if noread_pct >= 5.0 { Level::Error } else { Level::Warning }, code: "noread", title: format!("讀碼失敗 {noread} 件（{noread_pct}%）"), detail, action: action.into() });
    }

    // 讀到鄰件條碼被攔（REENTRY）：有就提，10 件以上紅
    if let Some(r) = o.reasons.iter().find(|r| r.key == "REENTRY") {
        if r.count > 0 {
            out.push(Finding {
                level: if r.count >= 10 { Level::Error } else { Level::Warning },
                code: "reentry",
                title: format!("讀到鄰件條碼 {} 件，已攔到異常口", r.count),
                detail: "同一條碼幾秒內又進線＝讀碼站把還在進料區那件的面單當成這件的".into(),
                action: "讀碼器的讀碼區域（ROI）排除畫面上緣進料區；投料時前後件別靠太近".into(),
            });
        }
    }

    // 格口完成後又進線：件沒落進去被撿回重投。每格 2% 或 20 件以上就提，4% 紅
    let mut refed: Vec<_> = o.by_chute.iter().filter(|c| c.refed_after > 0 && c.done > 0).map(|c| (c.code.clone(), c.refed_after, pct(c.refed_after, c.done))).collect();
    refed.sort_by(|a, b| b.1.cmp(&a.1));
    let bad: Vec<_> = refed.iter().filter(|(_, n, p)| *p >= 2.0 || *n >= 20).collect();
    if !bad.is_empty() {
        let worst = bad[0];
        let list = bad.iter().map(|(c, n, p)| format!("{c} {n} 件（{p}%）")).collect::<Vec<_>>().join("、");
        out.push(Finding {
            level: if worst.2 >= 4.0 { Level::Error } else { Level::Warning },
            code: "refed",
            title: format!("{} 完成後又進線 {} 件（{}%）", worst.0, worst.1, worst.2),
            detail: format!("這些件系統記「完成」但其實沒落進格口，被人撿回重投；各格：{list}"),
            action: "盯這幾格一個班次：是推包太早（件還沒在小車上穩住）還是落袋口回彈".into(),
        });
    }

    // 卡件：每千件 5 次以上提；某一模組佔三成以上點名
    if o.jams.total >= 5 && o.jams.per_thousand >= 5.0 {
        let hot = o.jams.by_module.iter().max_by_key(|k| k.count);
        let (detail, action) = match hot {
            Some(h) if pct(h.count, o.jams.total) >= 30.0 => (
                format!("{} 佔 {} 次（{}%），其他模組加起來才 {} 次", module_label(&h.key), h.count, pct(h.count, o.jams.total), o.jams.total - h.count),
                format!("先檢查{}的推包機構與落袋口", module_label(&h.key)),
            ),
            _ => ("沒有特別集中的模組".to_string(), "看堵塞後取走的件是不是同一類包材（軟袋、過大）".to_string()),
        };
        out.push(Finding { level: if o.jams.per_thousand >= 10.0 { Level::Error } else { Level::Warning }, code: "jams", title: format!("卡件 {} 次（每千件 {:.0} 次）", o.jams.total, o.jams.per_thousand), detail, action });
    }

    // 分揀機異常率：1.5% 黃、3% 紅
    let sorter_pct = pct(o.range.abnormal, total);
    if sorter_pct >= 1.5 {
        let parts: Vec<String> = o.by_status.iter().filter(|s| s.status != 3 && s.count > 0).map(|s| format!("{} {} 件", status_label(s.status), s.count)).collect();
        out.push(Finding {
            level: if sorter_pct >= 3.0 { Level::Error } else { Level::Warning },
            code: "sorter",
            title: format!("分揀機異常 {} 件（{sorter_pct}%）", o.range.abnormal),
            detail: parts.join("、"),
            action: "堵塞後取走與指令取消多半跟著卡件來；失去追蹤集中在同一段就是那段光電要查".into(),
        });
    }

    // 仲介機回傳：門市關轉這類 10 件以上提
    let mw: Vec<_> = o.reasons.iter().filter(|r| !matches!(r.key.as_str(), "NOREAD" | "REENTRY" | "OTHER") && r.defaulted > 0).collect();
    let mw_total: i64 = mw.iter().map(|r| r.defaulted).sum();
    if mw_total >= 10 {
        let list = mw.iter().map(|r| format!("{} {} 件", reason_label(&r.key), r.defaulted)).collect::<Vec<_>>().join("、");
        let late = o.reasons.iter().filter(|r| matches!(r.key.as_str(), "LATE" | "TIMEOUT" | "MW_UNREACHABLE")).map(|r| r.defaulted).sum::<i64>();
        out.push(Finding {
            level: if late >= 10 { Level::Warning } else { Level::Info },
            code: "middleware",
            title: format!("仲介機回傳異常 {mw_total} 件進了異常口"),
            detail: list,
            action: if late >= 10 { "回覆太晚／逾時偏多，看仲介機那台是不是忙或網路不穩".into() } else { "門市關轉、查無訂單這類在派車前就能攔，請上游處理".into() },
        });
    }

    // 投料空檔：5–30 秒的空檔累計超過跨度 15% 就提
    let f = &o.feeding;
    if f.span_min > 0.0 && f.short_gap_min >= 30.0 && f.short_gap_min / f.span_min >= 0.15 {
        out.push(Finding {
            level: Level::Info,
            code: "feeding",
            title: format!("投料空檔累計 {:.0} 分鐘（{} 次 5–30 秒沒件）", f.short_gap_min, f.short_gaps),
            detail: format!("整天跨度 {:.0} 分鐘，另有 {} 次 30 秒～5 分鐘的中斷（{:.0} 分鐘）；分揀機節拍約 1 秒一件，不是瓶頸", f.span_min, f.mid_gaps, f.mid_gap_min),
            action: "投料連續性是產能上限，補人或調整籠車進場節奏".into(),
        });
    }

    out.sort_by_key(|f| match f.level { Level::Error => 0, Level::Warning => 1, Level::Info => 2 });
    out
}

fn cause_label(code: &str) -> &str {
    match code {
        "no_code" => "讀碼器沒讀到碼",
        "neighbor" => "讀到鄰件的碼",
        "bad_code" => "讀到的不是單號",
        "no_frame" => "讀碼器沒回應",
        other => other,
    }
}

fn module_label(key: &str) -> String {
    key.strip_prefix('M').map(|n| format!("第 {n} 模組")).unwrap_or_else(|| key.to_string())
}

fn status_label(status: i64) -> &'static str {
    match status {
        4 => "失去追蹤",
        5 => "堵塞",
        6 => "堵塞後取走",
        7 => "指令取消",
        8 => "觸發異常",
        _ => "其他",
    }
}

fn reason_label(code: &str) -> &str {
    match code {
        "STORE_CLOSED" => "門市關轉",
        "NOT_FOUND" => "查無訂單",
        "STATUS_ABNORMAL" => "訂單狀態異常",
        "UNCONFIRMED" => "訂單未確認",
        "NO_CHANNEL" => "仲介機未給格口",
        "LATE" => "回覆太晚",
        "TIMEOUT" => "逾時未回",
        "MW_UNREACHABLE" => "仲介機連不上",
        "LABEL_FETCH_FAILED" => "面單下載失敗",
        "ERROR" => "錯誤",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::stats::{Bucket, ChuteCount, JamStats, KeyCount, ReasonCount, StatusCount};

    fn overview(total: i64) -> Overview {
        // 只填門檻會看的欄位，其餘空
        let db_free = Overview {
            from: "2026-09-21".into(),
            to: "2026-09-21".into(),
            retention_days: 15,
            parcels_since: None,
            kpi: crate::server::stats::Kpi { today: Bucket::default(), yesterday: Bucket::default(), last7: Bucket::default(), last30: Bucket::default() },
            range: Bucket { total, done: total, ..Default::default() },
            daily: vec![],
            hourly: vec![],
            heatmap: vec![],
            by_chute: vec![],
            by_source: vec![],
            by_status: vec![],
            travel: Default::default(),
            print: Default::default(),
            report: Default::default(),
            compare: vec![],
            reasons: vec![],
            jams: JamStats::default(),
            stages: vec![],
            travel_by_chute: vec![],
            devices: vec![],
            duplicates: Default::default(),
            by_cart: vec![],
            noread_by_length: vec![],
            noread_causes: vec![],
            feeding: Default::default(),
            printing_used: false,
        };
        db_free
    }

    #[test]
    fn 沒件的日子沒有問題點_正常的一天也沒有() {
        assert!(findings(&overview(0)).is_empty());
        let mut o = overview(5000);
        o.range.noread = 100; // 2%
        o.by_chute = vec![ChuteCount { code: "L1".into(), label: "".into(), total: 1000, done: 990, abnormal: 10, refed_after: 5 }]; // 0.5%
        assert!(findings(&o).is_empty(), "{:?}", findings(&o));
    }

    #[test]
    fn 九二一那天_讀碼失敗_又進線_卡件_鄰件條碼都要點出來_紅在前() {
        let mut o = overview(9234);
        o.range.noread = 338;
        o.range.abnormal = 197;
        o.by_chute = vec![
            ChuteCount { code: "L2".into(), label: "".into(), total: 1068, done: 1050, abnormal: 18, refed_after: 42 },
            ChuteCount { code: "L1".into(), label: "".into(), total: 1093, done: 1080, abnormal: 13, refed_after: 23 },
            ChuteCount { code: "L3".into(), label: "".into(), total: 1073, done: 1051, abnormal: 22, refed_after: 7 },
        ];
        o.jams = JamStats { total: 94, per_thousand: 10.2, by_module: vec![KeyCount { key: "M4".into(), count: 31 }, KeyCount { key: "M3".into(), count: 24 }], by_hour: vec![] };
        o.reasons = vec![ReasonCount { key: "NOREAD".into(), count: 337, defaulted: 337 }, ReasonCount { key: "REENTRY".into(), count: 17, defaulted: 17 }, ReasonCount { key: "STORE_CLOSED".into(), count: 19, defaulted: 19 }];
        o.by_status = vec![StatusCount { status: 3, count: 9037 }, StatusCount { status: 6, count: 94 }, StatusCount { status: 4, count: 59 }];
        o.noread_causes = vec![KeyCount { key: "no_code".into(), count: 253 }, KeyCount { key: "no_frame".into(), count: 33 }];
        let f = findings(&o);
        let codes: Vec<_> = f.iter().map(|x| x.code).collect();
        assert!(codes.contains(&"noread") && codes.contains(&"refed") && codes.contains(&"jams") && codes.contains(&"reentry") && codes.contains(&"sorter") && codes.contains(&"middleware"), "{codes:?}");
        // 紅的排前面：又進線 4% 與卡件每千件 10 次都是紅
        let first_info = f.iter().position(|x| x.level == Level::Info).unwrap();
        assert!(f[..first_info].iter().all(|x| x.level != Level::Info));
        assert_eq!(f[0].level, Level::Error);
        let refed = f.iter().find(|x| x.code == "refed").unwrap();
        assert!(refed.title.starts_with("L2 完成後又進線 42 件（4%）"), "{}", refed.title);
        let jams = f.iter().find(|x| x.code == "jams").unwrap();
        assert!(jams.detail.contains("第 4 模組 佔 31 次"), "{}", jams.detail);
        let noread = f.iter().find(|x| x.code == "noread").unwrap();
        assert!(noread.detail.contains("讀碼器沒讀到碼 253 件") && noread.action.contains("面單朝上"), "{} / {}", noread.detail, noread.action);
    }
}
