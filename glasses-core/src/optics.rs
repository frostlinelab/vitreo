//! 光学数学 —— 移植自 [m2-md/liquid-glass-refraction-shader](https://github.com/m2-md/liquid-glass-refraction-shader)（MIT）的 `optics.ts`。
//!
//! 纯数学、无 GPU 依赖。`glass.wgsl` 中的同名函数与本模块保持一致；
//! 若两者漂移，CPU 端的位移图（displacement map）与 shader 将产生不同的形状。

/// 三维向量（足够表达法线与光线方向）。
pub type Vec3 = [f32; 3];

/// 折射率：空气。物理常数。
pub const IOR_AIR: f32 = 1.0;
/// 折射率：水。物理常数。
pub const IOR_WATER: f32 = 1.333;
/// 折射率：亚克力。物理常数。
pub const IOR_ACRYLIC: f32 = 1.49;
/// 折射率：冕牌玻璃。物理常数，也是 [`crate::GlassStyle`] 的默认 IOR。
pub const IOR_CROWN_GLASS: f32 = 1.52;
/// 折射率：蓝宝石。物理常数。
pub const IOR_SAPPHIRE: f32 = 1.77;
/// 折射率：钻石。物理常数。
pub const IOR_DIAMOND: f32 = 2.417;

/// 与 GLSL / WGSL 内建 `refract(I, N, eta)` 行为完全一致。
///
/// `i` 为单位入射向量（指向表面），`n` 为单位法线（指向入射一侧），`eta = n₁ / n₂`。
/// 发生全内反射时返回零向量。
pub fn refract(i: Vec3, n: Vec3, eta: f32) -> Vec3 {
    let ni = i[0] * n[0] + i[1] * n[1] + i[2] * n[2];
    let k = 1.0 - eta * eta * (1.0 - ni * ni);
    if k < 0.0 {
        return [0.0, 0.0, 0.0];
    }
    let s = eta * ni + k.sqrt();
    [
        eta * i[0] - s * n[0],
        eta * i[1] - s * n[1],
        eta * i[2] - s * n[2],
    ]
}

/// 屏幕空间折射位移：视线垂直入射屏幕，穿过厚度为 `depth_px` 的介质后，
/// 在背景平面上落点偏移多少像素。
pub fn refract_offset_px(n: Vec3, eta: f32, depth_px: f32) -> [f32; 2] {
    let r = refract([0.0, 0.0, -1.0], n, eta);
    if r == [0.0, 0.0, 0.0] {
        return [0.0, 0.0];
    }
    let rz = r[2].abs().max(1e-3);
    [r[0] * depth_px / rz, r[1] * depth_px / rz]
}

/// `eta = n_from / n_to`。空气进入玻璃即 `ior_to_eta(1.0, 1.52)`。
pub fn ior_to_eta(from: f32, to: f32) -> f32 {
    from / to
}

/// 阿贝数展开：`V = (n_d - 1) / (n_F - n_C)`。
/// 返回蓝端（486 nm）与红端（656 nm）之间的折射率差。
pub fn abbe_spread(nd: f32, abbe: f32) -> f32 {
    (nd - 1.0) / abbe
}

/// `t = 0` 为红端，`t = 1` 为蓝端；两端之间的线性扫掠。
pub fn dispersed_ior(nd: f32, spread: f32, t: f32) -> f32 {
    nd + spread * (t - 0.5)
}

/// 正入射反射率：`((n₁ - n₂) / (n₁ + n₂))²`。
pub fn schlick_f0(n1: f32, n2: f32) -> f32 {
    let r = (n1 - n2) / (n1 + n2);
    r * r
}

/// Schlick 近似的 Fresnel 项。
pub fn fresnel_schlick(cos_theta: f32, f0: f32) -> f32 {
    let c = (1.0 - cos_theta).clamp(0.0, 1.0);
    f0 + (1.0 - f0) * c.powi(5)
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: Vec3 = [0.0, 0.0, 1.0];

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    mod refract_tests {
        use super::*;

        #[test]
        fn parallel_ray_is_unbent() {
            let r = refract([0.0, 0.0, -1.0], N, ior_to_eta(IOR_AIR, IOR_CROWN_GLASS));
            assert!(close(r[0], 0.0, 1e-6));
            assert!(close(r[1], 0.0, 1e-6));
            assert!(close(r[2], -1.0, 1e-6));
        }

        #[test]
        fn satisfies_snells_law() {
            let a = std::f32::consts::FRAC_PI_6; // 30°
            let i: Vec3 = [a.sin(), 0.0, -a.cos()];
            let eta = ior_to_eta(IOR_AIR, IOR_CROWN_GLASS);
            let r = refract(i, N, eta);
            let sin_t = (r[0] * r[0] + r[1] * r[1]).sqrt();
            assert!(close(sin_t, eta * a.sin(), 1e-6));
        }

        #[test]
        fn zero_vector_past_critical_angle() {
            let a = std::f32::consts::FRAC_PI_4;
            let i: Vec3 = [a.sin(), 0.0, -a.cos()];
            let r = refract(i, N, ior_to_eta(IOR_CROWN_GLASS, IOR_AIR)); // 玻璃 → 空气
            assert_eq!(r, [0.0, 0.0, 0.0]);
        }
    }

    mod refract_offset_tests {
        use super::*;

        const TILTED: Vec3 = [0.6, 0.0, 0.8]; // bevel 上典型法线

        fn mag(v: [f32; 2]) -> f32 {
            (v[0] * v[0] + v[1] * v[1]).sqrt()
        }

        #[test]
        fn zero_offset_on_flat_surface() {
            let off = refract_offset_px([0.0, 0.0, 1.0], ior_to_eta(1.0, IOR_CROWN_GLASS), 90.0);
            assert!(close(mag(off), 0.0, 1e-5));
        }

        #[test]
        fn offset_grows_with_ior() {
            let water = refract_offset_px(TILTED, ior_to_eta(1.0, IOR_WATER), 90.0);
            let glass = refract_offset_px(TILTED, ior_to_eta(1.0, IOR_CROWN_GLASS), 90.0);
            let diamond = refract_offset_px(TILTED, ior_to_eta(1.0, IOR_DIAMOND), 90.0);
            assert!(mag(water) < mag(glass));
            assert!(mag(glass) < mag(diamond));
        }

        #[test]
        fn offset_grows_linearly_with_depth() {
            let a = refract_offset_px(TILTED, ior_to_eta(1.0, IOR_CROWN_GLASS), 45.0);
            let b = refract_offset_px(TILTED, ior_to_eta(1.0, IOR_CROWN_GLASS), 90.0);
            assert!(close(b[0], a[0] * 2.0, 1e-4));
        }

        #[test]
        fn blue_bends_more_than_red() {
            // BK7：n_d = 1.5168，阿贝展开 ≈ 0.0081
            let red = refract_offset_px(TILTED, ior_to_eta(1.0, 1.5168 - 0.0081 / 2.0), 90.0);
            let blue = refract_offset_px(TILTED, ior_to_eta(1.0, 1.5168 + 0.0081 / 2.0), 90.0);
            assert!(mag(blue) > mag(red));
        }
    }

    mod material_constants {
        use super::*;

        #[test]
        fn air_glass_f0_is_about_four_percent() {
            assert!(close(schlick_f0(IOR_AIR, IOR_CROWN_GLASS), 0.0426, 1e-4));
        }

        #[test]
        fn bk7_abbe_spread_is_about_eight_thousandths() {
            // (1.5168 - 1) / 64.17 ≈ 0.0080536；参考实现用 toBeCloseTo(0.00805, 5)。
            assert!(close(abbe_spread(1.5168, 64.17), 0.00805, 5e-6));
        }

        #[test]
        fn fresnel_endpoints() {
            assert!(close(fresnel_schlick(1.0, 0.04), 0.04, 1e-6));
            assert!(close(fresnel_schlick(0.0, 0.04), 1.0, 1e-6));
        }
    }

    mod edge_cases {
        use super::*;

        #[test]
        fn zero_offset_on_total_internal_reflection() {
            // 掠射角附近的法线，玻璃 → 空气方向：refract() 归零。
            let grazing: Vec3 = [0.99, 0.0, (1.0f32 - 0.99 * 0.99).sqrt()];
            let off = refract_offset_px(grazing, ior_to_eta(IOR_CROWN_GLASS, IOR_AIR), 90.0);
            assert_eq!(off, [0.0, 0.0]);
        }

        #[test]
        fn air_to_glass_eta_is_reciprocal() {
            assert!(close(ior_to_eta(1.0, 1.5), 2.0 / 3.0, 1e-6));
            assert!(close(ior_to_eta(1.5, 1.0), 1.5, 1e-6));
        }

        #[test]
        fn blue_end_above_red_end() {
            let red = dispersed_ior(1.5168, 0.008, 0.0);
            let blue = dispersed_ior(1.5168, 0.008, 1.0);
            assert!(red < blue);
            assert!(close(blue - red, 0.008, 1e-6));
            assert!(close(dispersed_ior(1.5168, 0.008, 0.5), 1.5168, 1e-6));
        }

        #[test]
        fn fresnel_stays_in_range() {
            for i in 0..=20 {
                let f = fresnel_schlick(i as f32 / 20.0, 0.0426);
                assert!((0.0426 - 1e-6..=1.0 + 1e-6).contains(&f));
            }
        }
    }
}
