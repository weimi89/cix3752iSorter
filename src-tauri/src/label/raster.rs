//! 面單圖 → 1-bit 點陣（TSPL BITMAP 資料）。
//!
//! 規則沿用現場已印了幾十萬張的 Node 版：依 profile 尺寸（`PAPER-01#100*150` → 100×150mm）
//! 以 203 DPI（8 px/mm）換算目標像素，等比縮放置中補白，高度方向上下各留 2mm 白邊，
//! 灰階後門檻 200 二值化，白 = 1（不印）、黑 = 0（印）。

use image::{DynamicImage, GrayImage, Luma, imageops::FilterType};

pub const PX_PER_MM: u32 = 8;
/// 上下留白（mm）；目標高度極小（<150px）時不留
const PADDING_MM: u32 = 2;
const THRESHOLD: u8 = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub width_bytes: u32,
    /// 每列 `width_bytes` 位元組，MSB 為最左邊的像素；1 = 白（不印）
    pub bits: Vec<u8>,
}

impl Raster {
    pub fn mm_width(&self) -> u32 {
        (self.width as f64 / PX_PER_MM as f64).round() as u32
    }
    pub fn mm_height(&self) -> u32 {
        (self.height as f64 / PX_PER_MM as f64).round() as u32
    }
}

/// 從 profile 名稱取尺寸（mm），如 `PAPER-01#100*150` → (100, 150)
pub fn profile_size_mm(profile: &str) -> Option<(u32, u32)> {
    let (_, rest) = profile.split_once('#')?;
    let (w, h) = rest.split_once('*')?;
    let w: u32 = w.trim().parse().ok()?;
    let h: u32 = h.trim().chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()?;
    (w > 0 && h > 0).then_some((w, h))
}

/// 依 profile 縮放到標籤紙大小；解析不出尺寸就維持原圖
pub fn fit_to_profile(img: &DynamicImage, profile: Option<&str>) -> GrayImage {
    let gray = img.to_luma8();
    let Some((mm_w, mm_h)) = profile.and_then(profile_size_mm) else {
        return gray;
    };
    let target_w = mm_w * PX_PER_MM;
    let target_h = mm_h * PX_PER_MM;
    let pad_px = if target_h < 150 { 0 } else { PADDING_MM * PX_PER_MM };
    let inner_h = target_h.saturating_sub(pad_px * 2).max(1);

    // contain：等比縮到 target_w × inner_h 內
    let (iw, ih) = gray.dimensions();
    let scale = f64::min(target_w as f64 / iw as f64, inner_h as f64 / ih as f64);
    let nw = ((iw as f64 * scale).round() as u32).clamp(1, target_w);
    let nh = ((ih as f64 * scale).round() as u32).clamp(1, inner_h);
    let resized = image::imageops::resize(&gray, nw, nh, FilterType::Lanczos3);

    let mut canvas = GrayImage::from_pixel(target_w, target_h, Luma([255u8]));
    let x0 = (target_w - nw) / 2;
    let y0 = pad_px + (inner_h - nh) / 2;
    image::imageops::overlay(&mut canvas, &resized, x0 as i64, y0 as i64);
    canvas
}

/// 二值化並打包成 TSPL 點陣
pub fn to_raster(gray: &GrayImage) -> Raster {
    let (width, height) = gray.dimensions();
    let width_bytes = width.div_ceil(8);
    let mut bits = vec![0u8; (width_bytes * height) as usize];
    for y in 0..height {
        for bx in 0..width_bytes {
            let mut byte = 0u8;
            let mut mask = 0x80u8;
            for x in bx * 8..(bx + 1) * 8 {
                if x < width && gray.get_pixel(x, y).0[0] >= THRESHOLD {
                    byte |= mask;
                }
                mask >>= 1;
            }
            bits[(y * width_bytes + bx) as usize] = byte;
        }
    }
    Raster { width, height, width_bytes, bits }
}

/// 一步到位：圖檔位元組 → 點陣
pub fn render(bytes: &[u8], profile: Option<&str>) -> Result<Raster, image::ImageError> {
    let img = image::load_from_memory(bytes)?;
    Ok(to_raster(&fit_to_profile(&img, profile)))
}

/// 把點陣還原成灰階圖（除錯／網頁預覽用）
pub fn to_preview(r: &Raster) -> GrayImage {
    let mut img = GrayImage::from_pixel(r.width, r.height, Luma([0u8]));
    for y in 0..r.height {
        for x in 0..r.width {
            let b = r.bits[(y * r.width_bytes + x / 8) as usize];
            if b & (0x80 >> (x % 8)) != 0 {
                img.put_pixel(x, y, Luma([255u8]));
            }
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_尺寸解析() {
        assert_eq!(profile_size_mm("PAPER-01#100*150"), Some((100, 150)));
        assert_eq!(profile_size_mm("PAPER-01#95*195"), Some((95, 195)));
        assert_eq!(profile_size_mm("EPSON L6190"), None);
        assert_eq!(profile_size_mm("X#0*10"), None);
    }

    #[test]
    fn 縮放到標籤尺寸並留白() {
        // 400x600 的白圖，profile 100x150mm → 800x1200，上下各 16px 白邊
        let img = DynamicImage::ImageLuma8(GrayImage::from_pixel(400, 600, Luma([0u8])));
        let g = fit_to_profile(&img, Some("PAPER-01#100*150"));
        assert_eq!(g.dimensions(), (800, 1200));
        assert_eq!(g.get_pixel(400, 5).0[0], 255, "上方留白");
        assert_eq!(g.get_pixel(400, 1195).0[0], 255, "下方留白");
        assert_eq!(g.get_pixel(400, 600).0[0], 0, "內容區為黑");
        // 內容高度 1168 → 寬 400*(1168/600)=779 → 左右各 ~10px 白
        assert_eq!(g.get_pixel(2, 600).0[0], 255);
    }

    #[test]
    fn 沒有尺寸就維持原圖() {
        let img = DynamicImage::ImageLuma8(GrayImage::from_pixel(123, 45, Luma([0u8])));
        assert_eq!(fit_to_profile(&img, Some("EPSON")).dimensions(), (123, 45));
        assert_eq!(fit_to_profile(&img, None).dimensions(), (123, 45));
    }

    #[test]
    fn 點陣打包_白為1黑為0_msb在左() {
        let mut g = GrayImage::from_pixel(10, 1, Luma([255u8]));
        g.put_pixel(0, 0, Luma([0]));   // 最左黑
        g.put_pixel(9, 0, Luma([199]));  // 199 < 200 → 黑
        let r = to_raster(&g);
        assert_eq!((r.width, r.height, r.width_bytes), (10, 1, 2));
        assert_eq!(r.bits, vec![0b0111_1111, 0b1000_0000], "第 2 位元組：x=8 白、x=9 黑、其餘（超出寬度）為 0");
        assert_eq!((r.mm_width(), r.mm_height()), (1, 0));
    }

    #[test]
    fn 點陣往返() {
        let mut g = GrayImage::from_pixel(17, 3, Luma([255u8]));
        g.put_pixel(16, 2, Luma([0]));
        let r = to_raster(&g);
        let back = to_preview(&r);
        assert_eq!(back.get_pixel(16, 2).0[0], 0);
        assert_eq!(back.get_pixel(0, 0).0[0], 255);
    }
}

#[cfg(test)]
mod preview_dump {
    /// 手動檢視用：`CIX_PREVIEW_OUT=/tmp/x.png cargo test 匯出點陣預覽 -- --ignored`
    #[test]
    #[ignore]
    fn 匯出點陣預覽() {
        let Ok(out) = std::env::var("CIX_PREVIEW_OUT") else { return };
        let src = std::env::var("CIX_PREVIEW_SRC").ok();
        let bytes = match src {
            Some(p) => std::fs::read(p).unwrap(),
            None => {
                let mut img = image::GrayImage::from_pixel(600, 900, image::Luma([255u8]));
                for y in (50..850).step_by(60) { for x in 40..560 { for dy in 0..6 { img.put_pixel(x, y + dy, image::Luma([0])); } } }
                for x in 0..600 { img.put_pixel(x, 0, image::Luma([0])); img.put_pixel(x, 899, image::Luma([0])); }
                let mut buf = std::io::Cursor::new(Vec::new());
                image::DynamicImage::ImageLuma8(img).write_to(&mut buf, image::ImageFormat::Png).unwrap();
                buf.into_inner()
            }
        };
        let r = super::render(&bytes, Some("PAPER-01#100*150")).unwrap();
        super::to_preview(&r).save(&out).unwrap();
        println!("{}x{} → {out}", r.width, r.height);
    }
}
