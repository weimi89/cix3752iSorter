//! 光電檢查（IR）相關的指令與回覆解析（`docs/protocol-spec.md` §4、舊 `web/ir.go`）。
//!
//! - `Kd[` → 分揀機回 `~[<每台 8 位十六進位，空白分隔>]`：全 0 = 該台光電正常，否則有光電被遮蔽
//! - `_1{9` 進維修模式 → `U<m2>0 1;Y<m2>0 p1` 查單台每顆光電的讀值（多行，`<<<` 開始、`FFFFFFFF` 結束，
//!   行內 `名稱 = 值`）→ `_` 離開維修模式；讀值 < 1000 = 該顆光電此刻被遮蔽
//! - `KY<m2>0 m -999` 屏蔽整台光電、`-1` 解除

/// `~[…]` 內容 → 每台分揀機的狀態：0 沒回覆、1 正常、2 有光電被遮蔽
pub fn parse_kd(body: &str, number: usize) -> Vec<u8> {
    let groups: Vec<&str> = body.split_whitespace().collect();
    (0..number)
        .map(|i| match groups.get(i) {
            None | Some(&"") => 0,
            Some(g) if g.bytes().all(|b| b == b'0') => 1,
            Some(_) => 2,
        })
        .collect()
}

/// `p1` 回覆的行 → 每顆光電是否被遮蔽（讀值 < 1000）；不足 `ir_num` 顆的補 `None`（沒讀到）
pub fn parse_p1(lines: &[String], ir_num: usize) -> Vec<Option<bool>> {
    let mut values = Vec::new();
    for line in lines {
        if line.contains("<<<") || line.contains("FFFFFFFF") {
            continue;
        }
        // 舊程式：去掉 `_`、`=` 字元後以空白切；能轉成整數的才算讀值（名稱類的字串跳過，
        // 舊程式會把它們當 0 算成「遮蔽」）。實際回覆格式仍待正式機確認（protocol-spec §10）
        let cleaned: String = line.chars().filter(|&c| c != '_' && c != '=').collect();
        for tok in cleaned.split_whitespace() {
            if let Ok(v) = tok.parse::<i64>() {
                values.push(v);
            }
        }
    }
    (0..ir_num).map(|i| values.get(i).map(|v| *v < 1000)).collect()
}

pub fn enter_maintenance() -> &'static str {
    "_1{9"
}

pub fn leave_maintenance() -> &'static str {
    "_"
}

/// 查單台每顆光電讀值（m2 = 該台在集群內的編號 0–7）
pub fn query_p1(m2: u32) -> String {
    format!("U{m2}0 1;Y{m2}0 p1")
}

/// 屏蔽／解除屏蔽整台的光電
pub fn block(m2: u32, block: bool) -> String {
    format!("KY{m2}0 m {}", if block { -999 } else { -1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kd_回覆_每台狀態() {
        assert_eq!(parse_kd("00000000 00000000 00000010 00000000", 8), vec![1, 1, 2, 1, 0, 0, 0, 0]);
        assert_eq!(parse_kd("", 2), vec![0, 0]);
    }

    #[test]
    fn p1_回覆_讀值小於千為遮蔽() {
        let lines = vec!["<<< p1".to_string(), "ir_1 = 120 ir_2 = 4021".to_string(), "ir_3 = 999".to_string(), "FFFFFFFF".to_string()];
        assert_eq!(parse_p1(&lines, 4), vec![Some(true), Some(false), Some(true), None]);
    }

    #[test]
    fn 指令字串() {
        assert_eq!(query_p1(3), "U30 1;Y30 p1");
        assert_eq!(block(3, true), "KY30 m -999");
        assert_eq!(block(3, false), "KY30 m -1");
    }
}
