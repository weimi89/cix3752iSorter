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

    /// 舊程式的合法性規則：尾碼不可為 0、側別只能 1/2/3、集群索引不可為負。
    pub fn is_valid(self) -> bool {
        self.trail() != 0 && (1..=3).contains(&self.side()) && self.group() >= 0
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
}
