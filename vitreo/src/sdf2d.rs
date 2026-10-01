//! 2D SDF 与玻璃表面几何 —— 移植自 [m2-md/liquid-glass-refraction-shader](https://github.com/m2-md/liquid-glass-refraction-shader)（MIT）的 `sdf2d.ts`。
//!
//! 与 `glass.wgsl` 内的 `sd_rounded_box` / `glass_height` / `glass_normal` /
//! `union_sdf` / `union_normal` / `jelly_*` 一一对应，签名与参数顺序保持一致。

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

/// 两个 SDF 的并集：取 min。内部为负的区域合并，
/// 是多面板 Merge（并集融合）策略的几何基础。
pub fn sdf_union(a: f32, b: f32) -> f32 {
    a.min(b)
}

/// bevel 高度场：`d` 为到边缘的有符号距离（内部为负），`w` 为 bevel 宽度。
/// 边缘处返回 0，bevel 结束处返回 1（四分之一圆弧轮廓）。
pub fn glass_height(d: f32, w: f32) -> f32 {
    let x = (-d / w.max(1e-3)).clamp(0.0, 1.0);
    let t = 1.0 - x;
    (1.0 - t * t).max(0.0).sqrt()
}

/// 由任意 SDF 高度场的有限差分构造表面法线。
///
/// [`glass_normal`] 是它对单个圆角盒的特化；Merge 策略的并集法线
/// （WGSL `union_normal`）同样由它表达 —— 对并集 SDF 取差分。
pub fn glass_normal_sdf(
    p: [f32; 2],
    sdf: impl Fn([f32; 2]) -> f32,
    bevel: f32,
    thickness: f32,
    eps: f32,
) -> Vec3 {
    let hx = glass_height(sdf([p[0] + eps, p[1]]), bevel)
        - glass_height(sdf([p[0] - eps, p[1]]), bevel);
    let hy = glass_height(sdf([p[0], p[1] + eps]), bevel)
        - glass_height(sdf([p[0], p[1] - eps]), bevel);

    let k = thickness / (2.0 * eps);
    let x = -hx * k;
    let y = -hy * k;
    let len = (x * x + y * y + 1.0).sqrt();
    [x / len, y / len, 1.0 / len]
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
    glass_normal_sdf(p, |q| sd_rounded_box(q, half_size, radius), bevel, thickness, eps)
}

// ---------- 果冻形变（与 glass.wgsl 的 JELLY_* / jelly_* 对应） ----------
//
// 艺术性扩展，不属于物理光学推导：运动中的面板按挤压拉伸动画原理
// 沿速度方向伸长、垂直方向收缩，按压时整体微缩，bevel 法线滞后于运动。
// 常数与公式必须与 glass.wgsl 逐行一致（parity 不变量）。

/// 拉伸量增益：e = |velocity| * JELLY_STRETCH_GAIN（s/px，量纲为时间的倒数）。
pub const JELLY_STRETCH_GAIN: f32 = 4e-4;
/// 拉伸量上限（300 px/s 的运动即到达）。
pub const JELLY_STRETCH_MAX: f32 = 0.15;
/// 按压微缩：press = 1 时整体缩到 95%。
pub const JELLY_PRESS_SQUASH: f32 = 0.05;
/// 法线滞后倾斜增益（bevel 带内、按边缘权重加权）。
pub const JELLY_NORMAL_LAG: f32 = 0.9;

/// 把局部坐标 `p`（相对面板中心）映射进**未变形**盒子的采样空间：
/// 先按压均匀微缩（均匀缩放的 SDF 是精确的），再沿速度方向压缩采样坐标
/// （等效于轮廓沿速度拉伸、垂直收缩；距离按 [`jelly_scale`] 修正）。
///
/// 与 WGSL `jelly_sample` 逐行一致。
pub fn jelly_sample(p: [f32; 2], velocity: [f32; 2], press: f32) -> [f32; 2] {
    let k = 1.0 - JELLY_PRESS_SQUASH * press;
    let mut q = [p[0] / k, p[1] / k];
    let speed = (velocity[0] * velocity[0] + velocity[1] * velocity[1]).sqrt();
    if speed > 1e-3 {
        let dir = [velocity[0] / speed, velocity[1] / speed];
        let e = (speed * JELLY_STRETCH_GAIN).min(JELLY_STRETCH_MAX);
        let along = q[0] * dir[0] + q[1] * dir[1];
        let perp = -q[0] * dir[1] + q[1] * dir[0];
        // 旋转回面板坐标系再求 SDF（盒子半尺寸是轴对齐的）。
        let a = along / (1.0 + e);
        let b = perp / (1.0 - 0.5 * e);
        q = [dir[0] * a - dir[1] * b, dir[1] * a + dir[0] * b];
    }
    q
}

/// 变形 SDF 的距离修正系数：按压缩放 × 拉伸的垂直收缩量。
/// 零点（轮廓位置）不受它影响，只修正梯度量级以保持 AA 带宽近似不变。
/// 与 WGSL `jelly_scale` 逐行一致。
pub fn jelly_scale(velocity: [f32; 2], press: f32) -> f32 {
    let k = 1.0 - JELLY_PRESS_SQUASH * press;
    let speed = (velocity[0] * velocity[0] + velocity[1] * velocity[1]).sqrt();
    let e = (speed * JELLY_STRETCH_GAIN).min(JELLY_STRETCH_MAX);
    k * (1.0 - 0.5 * e)
}

/// 变形后的圆角盒 SDF：[`sd_rounded_box`] 的果冻版本。
/// 与 WGSL `jelly_sdf` 逐行一致。
pub fn jelly_sdf(
    p: [f32; 2],
    b: [f32; 2],
    r: f32,
    velocity: [f32; 2],
    press: f32,
) -> f32 {
    sd_rounded_box(jelly_sample(p, velocity, press), b, r) * jelly_scale(velocity, press)
}

/// 法线滞后倾斜：在 bevel 法线上叠加与速度**反向**的倾斜，
/// 让折射显得"跟不上"运动。`d` 是该像素的变形后 SDF（用于边缘权重：
/// 只倾斜 bevel 带，内部平台保持平坦），`bevel` 为倒角宽度。
/// 与 WGSL `jelly_lag` 逐行一致。
pub fn jelly_lag(n: Vec3, velocity: [f32; 2], d: f32, bevel: f32) -> Vec3 {
    let speed = (velocity[0] * velocity[0] + velocity[1] * velocity[1]).sqrt();
    if speed < 1e-3 {
        return n;
    }
    let dir = [velocity[0] / speed, velocity[1] / speed];
    let e = (speed * JELLY_STRETCH_GAIN).min(JELLY_STRETCH_MAX);
    let w = (1.0 - glass_height(d, bevel)) * JELLY_NORMAL_LAG;
    let x = n[0] - dir[0] * e * w;
    let y = n[1] - dir[1] * e * w;
    let len = (x * x + y * y + n[2] * n[2]).sqrt();
    [x / len, y / len, n[2] / len]
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

    // Merge（并集融合）策略的 parity：镜像 glass.wgsl 的 union_sdf / union_normal。
    mod merge_tests {
        use super::*;

        const BEVEL: f32 = 34.0;
        const THICKNESS: f32 = 6.0;
        const EPS: f32 = 1.0;

        // 两块左右重叠的圆角盒：A 中心 (0,0)，B 中心 (260,0)，
        // 半尺寸都是 (210,130)，圆角 40。重叠区 x ∈ [50, 210]。
        fn union_d(p: [f32; 2]) -> f32 {
            sdf_union(
                sd_rounded_box(p, [210.0, 130.0], 40.0),
                sd_rounded_box([p[0] - 260.0, p[1]], [210.0, 130.0], 40.0),
            )
        }

        #[test]
        fn union_contains_both_components() {
            for p in [[0.0, 0.0], [260.0, 0.0], [130.0, 0.0]] {
                let d = union_d(p);
                assert!(d < 0.0, "p = {p:?}, d = {d}");
            }
        }

        #[test]
        fn overlap_interior_is_flat() {
            // 重叠区中部：离两块面板的边缘都远于 bevel 宽度，
            // 并集法线应接近 (0,0,1) —— 融合后内部没有内边缘。
            let n = glass_normal_sdf([130.0, 0.0], union_d, BEVEL, THICKNESS, EPS);
            assert!(n[0].abs() < 1e-6, "n = {n:?}");
            assert!(n[1].abs() < 1e-6, "n = {n:?}");
        }

        #[test]
        fn inner_edge_of_overlap_is_absorbed() {
            // x = 60 在 B 的左缘（x = 50）内侧 10px —— 单面板法线在这里已经
            // 明显外倾，但并集在 x 方向远深于 bevel（A 的边缘在 x = 210），
            // 融合玻璃应当保持平坦：这正是 Merge 与逐层 Stack 的区别。
            let n = glass_normal_sdf([60.0, 0.0], union_d, BEVEL, THICKNESS, EPS);
            assert!(n[0].abs() < 1e-6, "n = {n:?}");
        }

        #[test]
        fn outer_bevel_survives_at_union_boundary() {
            // 并集外轮廓（B 的右缘 x = 470）的倒角环仍然存在且外倾。
            let n = glass_normal_sdf([460.0, 0.0], union_d, BEVEL, THICKNESS, EPS);
            assert!(n[0] > 0.15, "n = {n:?}");
        }

        #[test]
        fn union_normal_always_unit_length() {
            for p in [
                [0.0, 0.0],
                [60.0, 0.0],
                [130.0, 0.0],
                [130.0, 128.0],
                [460.0, 0.0],
                [465.0, 125.0],
            ] {
                let n = glass_normal_sdf(p, union_d, BEVEL, THICKNESS, EPS);
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                assert!((len - 1.0).abs() < 1e-5, "p = {p:?}, len = {len}");
            }
        }
    }

    // 果冻形变的 parity 测试：轮廓零点必须落在解析预期的位置上，
    // 法线滞后必须反向于运动方向且保持单位长度。
    mod jelly_tests {
        use super::*;

        const B: [f32; 2] = [100.0, 60.0];
        const R: f32 = 20.0;

        #[test]
        fn identity_at_rest() {
            // 零速度 + 零按压时，jelly_sdf 必须与普通 SDF 完全一致。
            for p in [[0.0, 0.0], [99.0, 0.0], [0.0, 59.0], [150.0, 0.0], [104.0, 63.0]] {
                let d = jelly_sdf(p, B, R, [0.0, 0.0], 0.0);
                let expected = sd_rounded_box(p, B, R);
                assert!((d - expected).abs() < 1e-5, "p = {p:?}, d = {d}");
            }
        }

        #[test]
        fn press_shrinks_silhouette_uniformly() {
            // press = 1 时均匀缩到 95%：右缘从 100 移到 95。
            let d_edge = jelly_sdf([95.0, 0.0], B, R, [0.0, 0.0], 1.0);
            assert!(d_edge.abs() < 1e-4, "edge d = {d_edge}");
            let d_in = jelly_sdf([0.0, 0.0], B, R, [0.0, 0.0], 1.0);
            assert!((d_in - (-57.0)).abs() < 1e-3, "center d = {d_in}");
            // 中间按压量线性插值：press = 0.5 → 97.5。
            let d_half = jelly_sdf([97.5, 0.0], B, R, [0.0, 0.0], 0.5);
            assert!(d_half.abs() < 1e-4, "half-press edge d = {d_half}");
        }

        #[test]
        fn stretch_elongates_along_velocity() {
            // v = 200 px/s → e = 0.08：y = 0 轴上的轮廓从 100 移到 108，
            // 垂直方向收缩 4%（1 - 0.5e）：轮廓从 60 移到 57.6。
            let v = [200.0, 0.0];
            let leading = jelly_sdf([108.0, 0.0], B, R, v, 0.0);
            assert!(leading.abs() < 0.01, "leading edge d = {leading}");
            let trailing = jelly_sdf([-108.0, 0.0], B, R, v, 0.0);
            assert!(trailing.abs() < 0.01, "trailing edge d = {trailing}");
            let side = jelly_sdf([0.0, B[1] * (1.0 - 0.5 * 0.08)], B, R, v, 0.0);
            assert!(side.abs() < 0.01, "perpendicular edge d = {side}");
        }

        #[test]
        fn stretch_is_capped() {
            // 极大速度下 e 钳制在 JELLY_STRETCH_MAX：轮廓最远 1.15 倍。
            let v = [10000.0, 0.0];
            let d = jelly_sdf([B[0] * (1.0 + JELLY_STRETCH_MAX), 0.0], B, R, v, 0.0);
            assert!(d.abs() < 0.01, "capped edge d = {d}");
        }

        #[test]
        fn stretch_grows_monotonically_with_speed() {
            // 固定采样点 (101, 0)（静止轮廓外 1px）：速度越快轮廓越远，
            // 该点的 SDF 值应单调递减（被越来越远的轮廓包进内侧）。
            let mut previous = f32::INFINITY;
            for speed in [50.0, 100.0, 150.0, 200.0, 300.0] {
                let d = jelly_sdf([101.0, 0.0], B, R, [speed, 0.0], 0.0);
                assert!(d < previous, "speed {speed}: d = {d}, previous = {previous}");
                previous = d;
            }
        }

        #[test]
        fn stretch_follows_velocity_direction() {
            // 垂直运动 (0, 200) → e = 0.08：y 轴上的轮廓从 60 移到 64.8。
            let v = [0.0, 200.0];
            let d = jelly_sdf([0.0, B[1] * 1.08], B, R, v, 0.0);
            assert!(d.abs() < 0.01, "vertical leading edge d = {d}");
            // 垂直于运动的方向收缩：x 边从 100 移到 96。
            let dx = jelly_sdf([100.0, 0.0], B, R, v, 0.0);
            assert!((dx - 4.0).abs() < 0.05, "perpendicular contraction d = {dx}");
        }

        #[test]
        fn lag_tilts_normal_against_motion() {
            // 右缘 bevel 带上，向右运动时法线 x 分量应减小（滞后 = 反向倾斜）。
            let p = [99.0, 0.0];
            let base = glass_normal(p, B, R, 34.0, 6.0, 1.0);
            let d = jelly_sdf(p, B, R, [200.0, 0.0], 0.0);
            let lagged = jelly_lag(base, [200.0, 0.0], d, 34.0);
            assert!(lagged[0] < base[0], "base nx = {}, lagged nx = {}", base[0], lagged[0]);
        }

        #[test]
        fn lag_is_identity_at_rest() {
            let n = [0.3, -0.4, 0.87];
            let out = jelly_lag(n, [0.0, 0.0], -5.0, 34.0);
            assert!((out[0] - n[0]).abs() < 1e-6);
            assert!((out[1] - n[1]).abs() < 1e-6);
            assert!((out[2] - n[2]).abs() < 1e-6);
        }

        #[test]
        fn lag_keeps_unit_length() {
            for p in [[0.0, 0.0], [99.0, 0.0], [0.0, 59.0], [104.0, 63.0], [-99.0, -30.0]] {
                let base = glass_normal(p, B, R, 34.0, 6.0, 1.0);
                let d = jelly_sdf(p, B, R, [-150.0, 90.0], 0.3);
                let n = jelly_lag(base, [-150.0, 90.0], d, 34.0);
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                assert!((len - 1.0).abs() < 1e-5, "p = {p:?}, len = {len}");
            }
        }

        #[test]
        fn lag_only_touches_bevel_band() {
            // 内部平台（离边缘远于 bevel 宽度）法线应保持 (0,0,1)。
            let n = [0.0, 0.0, 1.0];
            let out = jelly_lag(n, [300.0, 0.0], -80.0, 34.0);
            assert!(out[0].abs() < 1e-6, "interior nx = {}", out[0]);
        }
    }
}
