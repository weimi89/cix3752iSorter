//! 格口 CID：7 位十進位 `1G PP S TT`。
//!
//! - `1G`：10 + 分揀集群索引（現場單集群 → `10`）
//! - `PP`：沿線位置（0–99）
//! - `S`：側別 1/2/3
//! - `TT`：尾碼（現場 23 = 左、24 = 右、01 = NG）
//!
//! 例：`1004324` → 集群 0、位置 04、側 3、尾碼 24。詳見 `docs/protocol-spec.md` §5。

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cid(pub u32);

impl Cid {
    pub fn group(self) -> i32 {
        (self.0 / 100_000) as i32 - 10
    }

    pub fn pos(self) -> u32 {
        (self.0 / 1000) % 100
    }

    pub fn side(self) -> u32 {
        (self.0 / 100) % 10
    }

    pub fn trail(self) -> u32 {
        self.0 % 100
    }

    /// 舊程式的合法性規則：尾碼不可為 0、側別只能 1/2/3、集群索引不可為負；
    /// 另要求是 `1` 開頭的七位數——`Kn` 只帶位置、側別、尾碼，位數不對時拆出來的值沒有意義
    pub fn is_valid(self) -> bool {
        (1_000_000..=1_999_999).contains(&self.0) && self.trail() != 0 && (1..=3).contains(&self.side()) && self.group() >= 0
    }

    /// 資料庫與請求裡的 CID 是整數欄位
    /// 超出 u32 的值回 None（直接 `as u32` 會讓負數變大數、超大數繞回成看似合法的 CID）
    pub fn try_from_i64(v: i64) -> Option<Self> {
        u32::try_from(v).ok().map(Cid)
    }

    /// 由組成部分組回 CID
    pub fn from_parts(group: u32, pos: u32, side: u32, trail: u32) -> Self {
        Cid((10 + group) * 100_000 + pos * 1000 + side * 100 + trail)
    }
}

impl std::fmt::Display for Cid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 拆解現場格口() {
        let c = Cid(1004324);
        assert_eq!((c.group(), c.pos(), c.side(), c.trail()), (0, 4, 3, 24));
        assert!(c.is_valid());
        assert_eq!(Cid::from_parts(0, 4, 3, 24), c);
    }

    #[test]
    fn 預設口與直通口() {
        assert!(Cid(1007301).is_valid());
        assert!(Cid(1007323).is_valid());
    }

    #[test]
    fn 尾碼零或側別超界視為無效() {
        assert!(!Cid(1004300).is_valid());
        assert!(!Cid(1004024).is_valid());
        assert!(!Cid(1004424).is_valid());
        assert!(!Cid(904324).is_valid());
    }

    #[test]
    fn 不是七位數且一開頭的一律無效() {
        // 負數轉 u32 後的大數：群組、側別、尾碼都「看起來合法」，會組出莫名其妙的 Kn
        assert!(!Cid(-1i64 as u32).is_valid());
        assert!(!Cid(10_004_324).is_valid(), "八位數");
        assert!(!Cid(204_324).is_valid(), "六位數");
        assert!(!Cid(2_004_324).is_valid(), "2 開頭");
        assert!(Cid(1_999_323).is_valid(), "上界：群組 9、位置 99、側 3、尾碼 23");
    }

    #[test]
    fn 從資料庫或請求的整數轉換_超出範圍不可繞回() {
        assert_eq!(Cid::try_from_i64(1_004_324), Some(Cid(1_004_324)));
        assert_eq!(Cid::try_from_i64(-1), None);
        assert_eq!(Cid::try_from_i64((1i64 << 32) + 1_004_324), None, "不可繞回成合法 CID");
    }


    #[test]
    fn 編解碼往返_所有合法組合() {
        for group in 0..10 {
            for pos in 0..100 {
                for side in 1..=3 {
                    for trail in [1, 23, 24, 99] {
                        let c = Cid::from_parts(group, pos, side, trail);
                        assert!(c.is_valid(), "{c}");
                        assert_eq!((c.group(), c.pos(), c.side(), c.trail()), (group as i32, pos, side, trail), "{c}");
                    }
                }
            }
        }
    }

}
