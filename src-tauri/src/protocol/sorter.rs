//! 分揀機上行訊號解析（`docs/protocol-spec.md` §4）。

use super::{ints, strip_line};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SorterSignal {
    /// `~c<cart> <a> <b> 1`：指令受理，`Kn` 分配到小車
    C { cart: u32, a: i32, b: i32 },
    /// `~j<cart> 1`：收件
    J { cart: u32 },
    /// `~g<cart> <x> <y>`：離開頭部
    G { cart: u32, x: i32, y: i32 },
    /// `~n<cart>`：到達本集群尾部
    N { cart: u32 },
    /// `~e<cart>`：掉落完成
    E { cart: u32 },
    /// `~k<cart> <pos> <pos2>`：堵塞
    K { cart: u32, pos: i32, pos2: i32 },
    /// `~u<cart> <pos> <z>`：丟失／取走
    U { cart: u32, pos: i32, z: i32 },
    /// `~q<slot> <len> <cart>`：交接確認 [推斷]
    Q { slot: u32, len: i32, cart: i32 },
    /// `~f…`：小車互動診斷 [推斷]，原樣保留數值
    F(Vec<i32>),
    /// `~x<cart>`：取消回應／清除 [推斷]
    X { cart: i32 },
    /// `~y…`：光電位置診斷 [推斷]
    Y(Vec<i32>),
    /// `~I<n>`：入口光電觸發
    I { n: i32 },
    /// `~v<m2> <bits>`：輸入點狀態
    Input { m2: u32, bits: u8 },
    /// `~[…]`：`Kd[` 的光電狀態回覆，內容為 `]` 前的字串
    IrStatus(String),
    /// `~k-1 -1`：停止中
    Stopped,
    /// `p1` 查詢回覆的其中一行（`<<<` 開始、`FFFFFFFF` 結束，行內含 ` = `）
    P1Line(String),
    Unknown(String),
}

pub fn parse_sorter(raw: &str) -> Option<SorterSignal> {
    let line = strip_line(raw);
    if line.is_empty() {
        return None;
    }
    if !line.starts_with('~') || line.len() < 2 {
        if line.contains("<<<") || line.contains(" = ") || line.contains("FFFFFFFF") {
            return Some(SorterSignal::P1Line(line.to_string()));
        }
        return Some(SorterSignal::Unknown(line.to_string()));
    }
    let kind = line.as_bytes()[1] as char;
    let rest = &line[2..];

    if kind == '[' {
        let body = rest.split(']').next().unwrap_or("").trim();
        return Some(SorterSignal::IrStatus(body.to_string()));
    }

    let nums = ints(rest);
    let sig = match (kind, nums.as_slice()) {
        ('c', [cart, a, b, ..]) if *cart >= 0 => SorterSignal::C { cart: *cart as u32, a: *a, b: *b },
        ('j', [cart, ..]) if *cart >= 0 => SorterSignal::J { cart: *cart as u32 },
        ('g', [cart, x, y, ..]) if *cart >= 0 => SorterSignal::G { cart: *cart as u32, x: *x, y: *y },
        ('n', [cart, ..]) if *cart >= 0 => SorterSignal::N { cart: *cart as u32 },
        ('e', [cart, ..]) if *cart >= 0 => SorterSignal::E { cart: *cart as u32 },
        ('k', [-1, ..]) => SorterSignal::Stopped,
        ('k', [cart, pos, pos2, ..]) if *cart >= 0 => SorterSignal::K { cart: *cart as u32, pos: *pos, pos2: *pos2 },
        ('u', [cart, pos, z, ..]) if *cart >= 0 => SorterSignal::U { cart: *cart as u32, pos: *pos, z: *z },
        ('q', [slot, len, cart, ..]) if *slot >= 0 => SorterSignal::Q { slot: *slot as u32, len: *len, cart: *cart },
        ('f', _) => SorterSignal::F(nums),
        ('x', [cart, ..]) => SorterSignal::X { cart: *cart },
        ('y', _) => SorterSignal::Y(nums),
        ('I', [n, ..]) => SorterSignal::I { n: *n },
        ('I', []) => SorterSignal::I { n: 1 },
        ('v', [m2, bits, ..]) if *m2 >= 0 => SorterSignal::Input { m2: *m2 as u32, bits: (*bits & 0xff) as u8 },
        _ => SorterSignal::Unknown(line.to_string()),
    };
    Some(sig)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 正常件全流程訊號() {
        assert_eq!(parse_sorter("~c22 22 410 1"), Some(SorterSignal::C { cart: 22, a: 22, b: 410 }));
        assert_eq!(parse_sorter("~j22 1"), Some(SorterSignal::J { cart: 22 }));
        assert_eq!(parse_sorter("~g22 -1 1"), Some(SorterSignal::G { cart: 22, x: -1, y: 1 }));
        assert_eq!(parse_sorter("~e22"), Some(SorterSignal::E { cart: 22 }));
        assert_eq!(parse_sorter("~n5"), Some(SorterSignal::N { cart: 5 }));
    }

    #[test]
    fn 異常訊號() {
        assert_eq!(parse_sorter("~k23 73 73"), Some(SorterSignal::K { cart: 23, pos: 73, pos2: 73 }));
        assert_eq!(parse_sorter("~u23 73 -999"), Some(SorterSignal::U { cart: 23, pos: 73, z: -999 }));
        assert_eq!(parse_sorter("~k-1 -1"), Some(SorterSignal::Stopped));
        assert_eq!(parse_sorter("~I1"), Some(SorterSignal::I { n: 1 }));
    }

    #[test]
    fn 診斷類訊號保留數值() {
        assert_eq!(parse_sorter("~q24 25 22"), Some(SorterSignal::Q { slot: 24, len: 25, cart: 22 }));
        assert_eq!(parse_sorter("~f5 3 22 1 9"), Some(SorterSignal::F(vec![5, 3, 22, 1, 9])));
        assert_eq!(parse_sorter("~x13"), Some(SorterSignal::X { cart: 13 }));
        assert_eq!(parse_sorter("~y8 1 1 1 1"), Some(SorterSignal::Y(vec![8, 1, 1, 1, 1])));
        assert_eq!(parse_sorter("~v0 32"), Some(SorterSignal::Input { m2: 0, bits: 32 }));
    }

    #[test]
    fn 光電狀態與p1回覆() {
        assert_eq!(
            parse_sorter("~[00000183 00000000 00000000 00000020 00000000 00000000 00000000 00000000]"),
            Some(SorterSignal::IrStatus("00000183 00000000 00000000 00000020 00000000 00000000 00000000 00000000".into()))
        );
        assert_eq!(parse_sorter("<<< p1"), Some(SorterSignal::P1Line("<<< p1".into())));
        assert_eq!(parse_sorter("ir3 = 1"), Some(SorterSignal::P1Line("ir3 = 1".into())));
    }

    #[test]
    fn 殘缺行歸未知() {
        assert_eq!(parse_sorter("~c22"), Some(SorterSignal::Unknown("~c22".into())));
        assert_eq!(parse_sorter("~"), Some(SorterSignal::Unknown("~".into())));
        assert_eq!(parse_sorter("\t\n"), None);
    }
}
