//! 皮帶線上行訊號解析（`docs/protocol-spec.md` §2）。

use super::{ints, strip_line};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BeltSignal {
    /// `~P<slot> <n>`：觸發點，建立包裹
    P { slot: u32, n: i32 },
    /// `~L<slot> <len> <gap>`：長度與間距
    L { slot: u32, len: i32, gap: i32 },
    /// `~O<slot> <n>`：交接點，此刻決定格口
    O { slot: u32, n: i32 },
    /// `~E<slot> <n>`：離開皮帶
    E { slot: u32, n: i32 },
    /// `~k-1 -1`：皮帶停止中（週期性）
    Stopped,
    /// `~v<m2> <bits>`：輸入點狀態
    Input { m2: u32, bits: u8 },
    /// 認不得的行，原樣保留供日誌
    Unknown(String),
}

pub fn parse_belt(raw: &str) -> Option<BeltSignal> {
    let line = strip_line(raw);
    if line.len() < 2 || !line.starts_with('~') {
        return if line.is_empty() { None } else { Some(BeltSignal::Unknown(line.to_string())) };
    }
    // 依字元取，不可用位元組位置切：雜訊換成的替代字元佔 3 個位元組，切在中間會恐慌
    let mut chars = line[1..].chars();
    let kind = chars.next().unwrap_or('\0');
    let rest = chars.as_str();
    let nums = ints(rest);

    let sig = match (kind, nums.as_slice()) {
        ('P', [slot, n, ..]) if *slot >= 0 => BeltSignal::P { slot: *slot as u32, n: *n },
        ('L', [slot, len, gap, ..]) if *slot >= 0 => BeltSignal::L { slot: *slot as u32, len: *len, gap: *gap },
        ('O', [slot, n, ..]) if *slot >= 0 => BeltSignal::O { slot: *slot as u32, n: *n },
        ('E', [slot, n, ..]) if *slot >= 0 => BeltSignal::E { slot: *slot as u32, n: *n },
        ('k', _) => BeltSignal::Stopped,
        ('v', [m2, bits, ..]) if *m2 >= 0 => BeltSignal::Input { m2: *m2 as u32, bits: (*bits & 0xff) as u8 },
        _ => BeltSignal::Unknown(line.to_string()),
    };
    Some(sig)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 現場四個主訊號() {
        assert_eq!(parse_belt("~P24 1\n"), Some(BeltSignal::P { slot: 24, n: 1 }));
        assert_eq!(parse_belt("~L24 25 111"), Some(BeltSignal::L { slot: 24, len: 25, gap: 111 }));
        assert_eq!(parse_belt("~O24 1"), Some(BeltSignal::O { slot: 24, n: 1 }));
        assert_eq!(parse_belt("~E23 1"), Some(BeltSignal::E { slot: 23, n: 1 }));
    }

    #[test]
    fn 停止與輸入點() {
        assert_eq!(parse_belt("~k-1 -1"), Some(BeltSignal::Stopped));
        assert_eq!(parse_belt("~v0 32"), Some(BeltSignal::Input { m2: 0, bits: 32 }));
    }

    #[test]
    fn 欄位不足或負槽號歸入未知不恐慌() {
        assert_eq!(parse_belt("~P"), Some(BeltSignal::Unknown("~P".into())));
        assert_eq!(parse_belt("~L-1 3"), Some(BeltSignal::Unknown("~L-1 3".into())));
        assert_eq!(parse_belt("   "), None);
        assert_eq!(parse_belt("garbage"), Some(BeltSignal::Unknown("garbage".into())));
    }

    #[test]
    fn 波浪號後面是多位元組字元_歸入未知不恐慌() {
        // 線路雜訊的壞位元組會被換成替代字元（3 位元組），正好落在第二個字元的位置
        assert_eq!(parse_belt("~\u{FFFD}12 3"), Some(BeltSignal::Unknown("~\u{FFFD}12 3".into())));
        assert_eq!(parse_belt("~é1 1"), Some(BeltSignal::Unknown("~é1 1".into())));
        assert_eq!(parse_belt("~中"), Some(BeltSignal::Unknown("~中".into())));
    }

}
