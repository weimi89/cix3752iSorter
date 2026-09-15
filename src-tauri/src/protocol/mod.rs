//! 裝置文字行協定：純函式，無 I/O。規格見 `docs/protocol-spec.md`。

pub mod belt;
pub mod cid;
pub mod ir;
pub mod command;
pub mod sorter;

pub use belt::{BeltSignal, parse_belt};
pub use cid::Cid;
pub use sorter::{SorterSignal, parse_sorter};

/// 去掉行尾、Tab 與兩端空白（舊程式同樣處理）
pub(crate) fn strip_line(raw: &str) -> &str {
    raw.trim_matches(|c: char| c == '\n' || c == '\r' || c == '\t' || c == ' ')
}

/// 把 `"24 25 111"` 這類以空白分隔的整數全部解析；解析不了的欄位就停止。
pub(crate) fn ints(s: &str) -> Vec<i32> {
    let mut out = Vec::with_capacity(4);
    for tok in s.split_whitespace() {
        match tok.parse::<i32>() {
            Ok(v) => out.push(v),
            Err(_) => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ints_遇到非數字就停() {
        assert_eq!(ints("24 25 111"), vec![24, 25, 111]);
        assert_eq!(ints("-1 -1"), vec![-1, -1]);
        assert_eq!(ints("3 x 5"), vec![3]);
        assert_eq!(ints(""), Vec::<i32>::new());
    }
}
