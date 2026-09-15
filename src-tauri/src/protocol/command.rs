//! 下行指令組裝（`docs/protocol-spec.md` §3、§5、§6）。
//!
//! 所有指令都不含行尾；送線時由 `frame()` 補 `;\n`（已以 `;` 結尾的只補 `\n`）。

use super::cid::Cid;

/// 分揀指令：`Kn 101 01 <pos><side> <speed> <trail> 1`。
/// `negative_trail` 為多集群且設備數 >2 時的用法（現場單集群不用）。
pub fn kn(cid: Cid, speed: u32, negative_trail: bool) -> String {
    let trail = cid.trail() as i32 * if negative_trail { -1 } else { 1 };
    format!("Kn 101 01 {}{} {} {} 1", cid.pos(), cid.side(), speed, trail)
}

/// 取消指令
pub fn kx(cart: u32) -> String {
    format!("Kx {cart}")
}

/// 允許小車離開本集群
pub fn ka(cart: u32) -> String {
    format!("Ka {cart}")
}

/// 連線後重置分揀機
pub fn reset_sorter() -> &'static str {
    "Kx999;Kk2"
}

/// 燈控：`rgb` 三位各為 0 暗／1 亮／2 閃（順序紅綠藍）
pub fn kl(m2: u32, led: u32, rgb: &str) -> String {
    format!("KL {m2} {led} 3{rgb}")
}

/// 查詢光電狀態（回 `~[…]`）
pub fn kd() -> &'static str {
    "Kd["
}

/// 查單台光電細節
pub fn p1(n: u32) -> String {
    format!("p1 {n}")
}

/// 補上行尾，成為可直接寫入 socket 的一行
pub fn frame(cmd: &str) -> String {
    let cmd = cmd.trim();
    if cmd.ends_with(';') { format!("{cmd}\n") } else { format!("{cmd};\n") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kn_對照現場日誌() {
        // cmd.log：[1788165552] Kn 101 01 43 80 24 1（當時速度 80）
        assert_eq!(kn(Cid(1004324), 80, false), "Kn 101 01 43 80 24 1");
        assert_eq!(kn(Cid(1002323), 100, false), "Kn 101 01 23 100 23 1");
        assert_eq!(kn(Cid(1004324), 80, true), "Kn 101 01 43 80 -24 1");
    }

    #[test]
    fn 其他指令() {
        assert_eq!(kx(13), "Kx 13");
        assert_eq!(ka(7), "Ka 7");
        assert_eq!(kl(0, 2, "200"), "KL 0 2 3200");
        assert_eq!(p1(3), "p1 3");
    }

    #[test]
    fn 行尾補齊() {
        assert_eq!(frame("KM998 3"), "KM998 3;\n");
        assert_eq!(frame("Kx999;Kk2"), "Kx999;Kk2;\n");
        assert_eq!(frame("KM999;"), "KM999;\n");
        assert_eq!(frame("  Kd[ "), "Kd[;\n");
    }
}
