//! 图片背景：解码 PNG/JPG，cover 模式裁剪到窗口尺寸（保持纵横比、居中、填满）。

use std::path::Path;

use image::imageops::{crop_imm, resize, FilterType};

/// 解码图片文件为 RGBA。
pub fn load_image_rgba(path: &Path) -> Result<image::RgbaImage, String> {
    image::open(path)
        .map(|img| img.to_rgba8())
        .map_err(|e| format!("无法解码 {}: {e}", path.display()))
}

/// cover 裁剪区域：源图中与目标等比例的最大矩形，居中放置。
///
/// 返回 `(x, y, w, h)`。源图比目标"更宽"时满高裁宽，反之满宽裁高；
/// 源图小于目标时取全图（后续放大）。整数运算，u64 防乘法溢出。
pub fn cover_crop_rect(iw: u32, ih: u32, tw: u32, th: u32) -> (u32, u32, u32, u32) {
    let (iw, ih, tw, th) = (iw as u64, ih as u64, tw as u64, th as u64);
    let (cw, ch) = if iw * th >= ih * tw {
        let cw = ih.saturating_mul(tw).div_ceil(th).min(iw);
        (cw as u32, ih as u32)
    } else {
        let ch = iw.saturating_mul(th).div_ceil(tw).min(ih);
        (iw as u32, ch as u32)
    };
    let x = (iw as u32 - cw) / 2;
    let y = (ih as u32 - ch) / 2;
    (x, y, cw, ch)
}

/// 把图片调整为恰好覆盖 `(tw, th)` 的 RGBA 字节（可直接喂 `Backdrop::from_rgba`）。
pub fn fit_cover(img: &image::RgbaImage, tw: u32, th: u32) -> Vec<u8> {
    let (iw, ih) = (img.width(), img.height());
    let (x, y, cw, ch) = cover_crop_rect(iw, ih, tw, th);
    if cw == tw && ch == th {
        // 比例已一致：直接取中心区域，避免无谓的重采样
        crop_imm(img, x, y, cw, ch).to_image().into_raw()
    } else {
        let crop = crop_imm(img, x, y, cw, ch);
        resize(crop.inner(), tw, th, FilterType::CatmullRom).into_raw()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_wide_source_keeps_full_height() {
        // 2000x1000 -> 1000x1000：满高裁宽，居中
        assert_eq!(
            cover_crop_rect(2000, 1000, 1000, 1000),
            (500, 0, 1000, 1000)
        );
    }

    #[test]
    fn crop_tall_source_keeps_full_width() {
        // 1000x2000 -> 1000x1000：满宽裁高，居中
        assert_eq!(
            cover_crop_rect(1000, 2000, 1000, 1000),
            (0, 500, 1000, 1000)
        );
    }

    #[test]
    fn same_ratio_takes_whole_image() {
        assert_eq!(cover_crop_rect(800, 600, 400, 300), (0, 0, 800, 600));
    }

    #[test]
    fn rounding_never_exceeds_source() {
        // 7x5 -> 3x1：ceil 得 ch=3，不越界，居中 y=1
        assert_eq!(cover_crop_rect(7, 5, 3, 1), (0, 1, 7, 3));
        // 5x7 -> 1x3：ceil 得 cw=3，居中 x=1
        assert_eq!(cover_crop_rect(5, 7, 1, 3), (1, 0, 3, 7));
    }

    #[test]
    fn small_source_takes_whole_image_for_upscale() {
        // 1:1 源 -> 4:3 目标：满宽裁高，100x75 垂直居中（y = (100-75)/2）
        assert_eq!(cover_crop_rect(100, 100, 800, 600), (0, 12, 100, 75));
    }

    #[test]
    fn fit_cover_output_size_matches_target() {
        let img = image::RgbaImage::from_pixel(300, 100, image::Rgba([200, 100, 50, 255]));
        let out = fit_cover(&img, 64, 64);
        assert_eq!(out.len(), 64 * 64 * 4);
    }
}
