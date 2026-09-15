//! TSPL 列印指令組裝（TSC 系列標籤機）。
//!
//! 位元組佈局與現場 Node 版一致：512 個 0x00 前導（喚醒／清空印表機緩衝）→ `SIZE` → profile 指令
//! → `CLS` → `BITMAP x,y,widthBytes,height,0,<data>` → `PRINT 1,1`。

use super::raster::Raster;
use crate::config::PrintProfile;

pub fn build(r: &Raster, profile: &PrintProfile) -> Vec<u8> {
    let mut out = Vec::with_capacity(512 + 128 + r.bits.len() + 16);
    out.extend_from_slice(&[0u8; 512]);
    out.extend_from_slice(format!("\r\nSIZE {} mm,{} mm", r.mm_width(), r.mm_height()).as_bytes());
    out.extend_from_slice(format!("\r\n{}", profile.cmd).as_bytes());
    out.extend_from_slice(b"\r\nCLS");
    out.extend_from_slice(format!("\r\nBITMAP {},{},{},0,", profile.org, r.width_bytes, r.height).as_bytes());
    out.extend_from_slice(&r.bits);
    out.extend_from_slice(b"\r\nPRINT 1,1\r\n");
    out
}

/// 從 `build` 產出的 TSPL 位元組還原點陣（列印任務頁預覽用）；找不到 `BITMAP` 標頭回 None
pub fn parse_bitmap(data: &[u8]) -> Option<Raster> {
    let key = b"\r\nBITMAP ";
    let start = data.windows(key.len()).position(|w| w == key)? + key.len();
    // 標頭：x,y,widthBytes,height,mode,
    let mut fields = Vec::with_capacity(5);
    let mut pos = start;
    while fields.len() < 5 {
        let end = data[pos..].iter().position(|&b| b == b',')? + pos;
        fields.push(std::str::from_utf8(&data[pos..end]).ok()?.trim().parse::<u32>().ok()?);
        pos = end + 1;
    }
    let (width_bytes, height) = (fields[2], fields[3]);
    let len = (width_bytes * height) as usize;
    let bits = data.get(pos..pos + len)?.to_vec();
    Some(Raster { width: width_bytes * 8, height, width_bytes, bits })
}

/// 測試頁：純文字，確認該台印表機活著
pub fn test_page(text: &str) -> Vec<u8> {
    let safe: String = text.chars().filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '-').collect();
    format!("CLS\r\nBOX 15,25,450,290,5\r\nTEXT 120,50,\"TSS24.BF2\",0,1,1,\"{safe} TEST OK!\"\r\nPRINT 1\r\n").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 指令佈局() {
        let r = Raster { width: 16, height: 2, width_bytes: 2, bits: vec![0xff, 0x00, 0xaa, 0x55] };
        let p = PrintProfile { cmd: "DIRECTION 1\r\nSPEED 6\r\nDENSITY 11".into(), org: "0,0".into(), retry: 30, retry_interval_ms: 200 };
        let out = build(&r, &p);
        assert!(out[..512].iter().all(|&b| b == 0));
        let head = String::from_utf8_lossy(&out[512..512 + 90]).into_owned();
        assert!(head.starts_with("\r\nSIZE 2 mm,0 mm\r\nDIRECTION 1\r\nSPEED 6\r\nDENSITY 11\r\nCLS\r\nBITMAP 0,0,2,2,0,"), "{head:?}");
        let data_start = 512 + "\r\nSIZE 2 mm,0 mm\r\nDIRECTION 1\r\nSPEED 6\r\nDENSITY 11\r\nCLS\r\nBITMAP 0,0,2,2,0,".len();
        assert_eq!(&out[data_start..data_start + 4], &[0xff, 0x00, 0xaa, 0x55]);
        assert!(out.ends_with(b"\r\nPRINT 1,1\r\n"));
    }

    #[test]
    fn 從指令還原點陣() {
        let r = Raster { width: 16, height: 2, width_bytes: 2, bits: vec![0xff, 0x00, 0xaa, 0x55] };
        let out = build(&r, &PrintProfile::default());
        let back = parse_bitmap(&out).unwrap();
        assert_eq!((back.width, back.height, back.width_bytes), (16, 2, 2));
        assert_eq!(back.bits, r.bits);
        assert!(parse_bitmap(b"CLS\r\nPRINT 1\r\n").is_none());
    }

    #[test]
    fn 測試頁不含可破壞指令的字元() {
        let t = String::from_utf8(test_page("L1\"\r\nPRINT 99")).unwrap();
        assert!(t.contains("\"L1PRINT 99 TEST OK!\""));
        assert_eq!(t.matches("PRINT").count(), 2);
    }
}
