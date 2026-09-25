//! 2D SDF 与玻璃表面几何 —— 移植自 [m2-md/liquid-glass-refraction-shader](https://github.com/m2-md/liquid-glass-refraction-shader)（MIT）的 `sdf2d.ts`。
//!
//! 与 `glass.wgsl` 内的 `sd_rounded_box` / `glass_height` / `glass_normal` 一一对应，
//! 签名与参数顺序保持一致。

use crate::optics::Vec3;

/// 圆角矩形 SDF（signed distance field）。
///
/// `p` 为相对面板中心的坐标，`b` 为半尺寸，`r` 为圆角半径。内部为负。
pub fn sd_rounded_box(p: [f32; 2], b: [f32; 2], r: f32) -> f32 {
    let qx = p[0].abs() - b[0] + r;
    let qy = p[1].abs() - b[1] + r;
    let inner = qx.max(qy).min(0.0);
    let outer = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    inner + outer - r
}

/// bevel 高度场：`d` 为到边缘的有符号距离（内部为负），`w` 为 bevel 宽度。
/// 边缘处返回 0，bevel 结束处返回 1（四分之一圆弧轮廓）。
pub fn glass_height(d: f32, w: f32) -> f32 {
    let x = (-d / w.max(1e-3)).clamp(0.0, 1.0);
    let t = 1.0 - x;
    (1.0 - t * t).max(0.0).sqrt()
}

/// 由高度场的有限差分构造表面法线。
///
/// `thickness` 是 bevel 剖面的高度（控制法线倾角），`eps` 为差分步长。
pub fn glass_normal(
    p: [f32; 2],
    half_size: [f32; 2],
    radius: f32,
    bevel: f32,
    thickness: f32,
    eps: f32,
) -> Vec3 {
    let hx = glass_height(sd_rounded_box([p[0] + eps, p[1]], half_size, radius), bevel)
        - glass_height(sd_rounded_box([p[0] - eps, p[1]], half_size, radius), bevel);
    let hy = glass_height(sd_rounded_box([p[0], p[1] + eps], half_size, radius), bevel)
        - glass_height(sd_rounded_box([p[0], p[1] - eps], half_size, radius), bevel);

    let k = thickness / (2.0 * eps);
    let x = -hx * k;
    let y = -hy * k;
    let len = (x * x + y * y + 1.0).sqrt();
    [x / len, y / len, 1.0 / len]
}

#[cfg(test)]
mod tests {
    use super::*;

    mod sd_rounded_box_tests {
        use super::*;

        #[test]
        fn negative_at_center() {
            assert!((sd_rounded_box([0.0, 0.0], [100.0, 60.0], 20.0) + 60.0).abs() < 1e-4);
        }

        #[test]
        fn zero_on_edge() {
            assert!((sd_rounded_box([100.0, 0.0], [100.0, 60.0], 20.0)).abs() < 1e-4);
            assert!((sd_rounded_box([0.0, 60.0], [100.0, 60.0], 20.0)).abs() < 1e-4);
        }

        #[test]
        fn corner_pulled_in_by_radius() {
            let d = sd_rounded_box([100.0, 60.0], [100.0, 60.0], 20.0);
            let expected = 20.0 * std::f32::consts::SQRT_2 - 20.0;
            assert!((d - expected).abs() < 1e-4);
        }

        #[test]
        fn sharp_box_when_radius_is_zero() {
            assert!((sd_rounded_box([100.0, 60.0], [100.0, 60.0], 0.0)).abs() < 1e-4);
            assert!((sd_rounded_box([103.0, 64.0], [100.0, 60.0], 0.0) - 5.0).abs() < 1e-4);
            assert!((sd_rounded_box([0.0, 0.0], [100.0, 60.0], 0.0) + 60.0).abs() < 1e-4);
        }
    }

    mod glass_height_tests {
        use super::*;

        #[test]
        fn zero_at_edge_one_at_bevel_end() {
            assert!((glass_height(0.0, 34.0)).abs() < 1e-6);
            assert!((glass_height(-34.0, 34.0) - 1.0).abs() < 1e-6);
            assert!((glass_height(-200.0, 34.0) - 1.0).abs() < 1e-6);
        }

        #[test]
        fn zero_outside_panel() {
            assert!((glass_height(12.0, 34.0)).abs() < 1e-6);
        }

        #[test]
        fn monotonically_increasing_inward() {
            let mut previous = glass_height(0.0, 34.0);
            let mut d = -1.0;
            while d >= -34.0 {
                let current = glass_height(d, 34.0);
                assert!(current > previous);
                previous = current;
                d -= 1.0;
            }
        }
    }

    mod glass_normal_tests {
        use super::*;

        const HALF: [f32; 2] = [210.0, 130.0];

        #[test]
        fn flat_on_plateau() {
            let n = glass_normal([0.0, 0.0], HALF, 40.0, 34.0, 6.0, 1.0);
            assert!(n[0].abs() < 1e-6);
            assert!(n[1].abs() < 1e-6);
            assert!((n[2] - 1.0).abs() < 1e-6);
        }

        #[test]
        fn tilts_outward_on_bevel_and_stays_symmetric() {
            let right = glass_normal([200.0, 0.0], HALF, 40.0, 34.0, 6.0, 1.0);
            let left = glass_normal([-200.0, 0.0], HALF, 40.0, 34.0, 6.0, 1.0);
            assert!(right[0] > 0.15);
            assert!((right[0] + left[0]).abs() < 1e-6);
        }

        #[test]
        fn tilts_more_as_thickness_grows() {
            let thin = glass_normal([200.0, 0.0], HALF, 40.0, 34.0, 3.0, 1.0);
            let thick = glass_normal([200.0, 0.0], HALF, 40.0, 34.0, 12.0, 1.0);
            assert!(thick[0] > thin[0]);
        }

        #[test]
        fn always_unit_length() {
            for p in [
                [0.0, 0.0],
                [200.0, 0.0],
                [0.0, 125.0],
                [205.0, 127.0],
                [400.0, 0.0],
                [-208.0, -128.0],
            ] {
                let n = glass_normal(p, HALF, 40.0, 34.0, 6.0, 1.0);
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                assert!((len - 1.0).abs() < 1e-5, "p = {p:?}, len = {len}");
            }
        }

        #[test]
        fn flat_outside_panel() {
            let n = glass_normal([400.0, 0.0], HALF, 40.0, 34.0, 6.0, 1.0);
            assert!(n[0].abs() < 1e-6);
            assert!(n[1].abs() < 1e-6);
            assert!((n[2] - 1.0).abs() < 1e-6);
        }
    }
}
