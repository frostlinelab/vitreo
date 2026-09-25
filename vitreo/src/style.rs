//! 玻璃材质参数。

use crate::optics::{abbe_spread, IOR_CROWN_GLASS};

/// 玻璃投影阴影。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowStyle {
    /// 阴影不透明度，`0.0` 关闭阴影。
    pub opacity: f32,
    /// 阴影模糊半径（像素）。
    pub blur: f32,
    /// 阴影偏移（像素，y 向下）。
    pub offset: [f32; 2],
}

impl Default for ShadowStyle {
    fn default() -> Self {
        Self {
            opacity: 0.0,
            blur: 30.0,
            offset: [0.0, 15.0],
        }
    }
}

/// 一块玻璃的全部材质参数。
///
/// `thickness` 与 `depth` 是两个不同的物理量（与 m2-md 的 uniform 划分一致）：
/// - `thickness`：bevel 剖面的**高度**，决定表面法线倾角；
/// - `depth`：光线在玻璃中穿行的**光程**，决定折射位移的幅度。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassStyle {
    /// 折射率。真实材料常数，默认 [`IOR_CROWN_GLASS`](crate::optics::IOR_CROWN_GLASS)（1.52）。
    pub ior: f32,
    /// bevel 宽度（像素）：边缘弯曲区的宽度。
    pub bevel: f32,
    /// bevel 剖面高度（像素）：法线倾角强度。
    pub thickness: f32,
    /// 光程（像素）：折射位移幅度。
    pub depth: f32,
    /// 色散强度，即蓝红两端 IOR 差。
    /// 物理值可用 `abbe_spread(1.5168, 64.17) ≈ 0.008`（BK7）；
    /// 默认 `0.15` 为参考实现的艺术化取值，肉眼可见的彩虹边缘。
    pub dispersion: f32,
    /// 背景模糊半径（像素），`0.0` 不模糊。
    pub blur: f32,
    /// 镜面高光强度。
    pub specular: f32,
    /// 玻璃染色颜色（同时也是 Fresnel 边缘辉光的颜色）。
    pub tint: [f32; 3],
    /// 染色不透明度，`0.0` 为纯光学（不染色）。
    pub tint_opacity: f32,
    /// 投影阴影。
    pub shadow: ShadowStyle,
}

impl Default for GlassStyle {
    fn default() -> Self {
        Self {
            ior: IOR_CROWN_GLASS,
            bevel: 34.0,
            thickness: 6.0,
            depth: 90.0,
            dispersion: 0.15,
            blur: 0.0,
            specular: 0.85,
            tint: [1.0, 1.0, 1.0],
            tint_opacity: 0.0,
            shadow: ShadowStyle::default(),
        }
    }
}

impl GlassStyle {
    /// 由真实材料的折射率与阿贝数构造物理精确的色散。
    ///
    /// ```text
    /// GlassStyle { ior: 1.5168, dispersion: GlassStyle::bk7_dispersion(), .. }
    /// ```
    pub fn bk7_dispersion() -> f32 {
        abbe_spread(1.5168, 64.17)
    }
}
