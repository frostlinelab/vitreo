//! 玻璃面板：几何 + 材质。

use crate::optics::{schlick_f0, IOR_AIR};
use crate::sdf2d::sd_rounded_box;
use crate::style::GlassStyle;

/// 一块玻璃面板。
///
/// 坐标均为物理像素，原点在视口左上角，y 向下（与 wgpu 帧缓冲、winit 光标一致）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassPanel {
    /// 中心坐标（像素）。
    pub center: [f32; 2],
    /// 完整尺寸（像素）。
    pub size: [f32; 2],
    /// 圆角半径（像素）。
    pub corner_radius: f32,
    /// 运动速度（像素/秒）——果冻形变的驱动量：轮廓沿速度方向拉伸、
    /// bevel 法线滞后倾斜。静态用法保持零值。
    pub velocity: [f32; 2],
    /// 按压量 0..1——按住拖拽时玻璃整体微缩（见 `sdf2d::JELLY_PRESS_SQUASH`）。
    pub press: f32,
    /// 材质参数。
    pub style: GlassStyle,
}

impl GlassPanel {
    /// 静态面板的便捷构造：零速度、零按压、默认材质。
    pub fn new(center: [f32; 2], size: [f32; 2], corner_radius: f32) -> Self {
        Self {
            center,
            size,
            corner_radius,
            velocity: [0.0; 2],
            press: 0.0,
            style: GlassStyle::default(),
        }
    }

    /// 半尺寸。
    pub fn half_size(&self) -> [f32; 2] {
        [self.size[0] * 0.5, self.size[1] * 0.5]
    }

    /// 点是否在面板内（圆角盒 SDF 命中测试）。
    pub fn contains(&self, point: [f32; 2]) -> bool {
        let p = [point[0] - self.center[0], point[1] - self.center[1]];
        sd_rounded_box(p, self.half_size(), self.corner_radius) < 0.0
    }

    pub(crate) fn to_uniform(self) -> PanelUniform {
        let style = &self.style;
        PanelUniform {
            center: self.center,
            half_size: self.half_size(),
            corner_radius: self.corner_radius,
            bevel: style.bevel,
            thickness: style.thickness,
            depth: style.depth,
            ior: style.ior,
            dispersion: style.dispersion,
            blur: style.blur,
            specular: style.specular,
            fresnel_f0: schlick_f0(IOR_AIR, style.ior),
            velocity: self.velocity,
            press: self.press,
            tint: [
                style.tint[0],
                style.tint[1],
                style.tint[2],
                style.tint_opacity,
            ],
            shadow: [
                style.shadow.opacity,
                style.shadow.blur,
                style.shadow.offset[0],
                style.shadow.offset[1],
            ],
        }
    }
}

/// `glass.wgsl` 中 `Panel` 结构的 Rust 镜像。
///
/// 布局约定：只有标量与 `[f32; 4]`，`[f32; 4]` 落在 16 字节对齐的偏移上，
/// 结构体总大小 96 字节（uniform 数组步长须为 16 的倍数）。
/// `velocity` + `press` 恰好占用原 `_pad` 的 12 字节（52..64）；
/// WGSL 侧把 velocity 拆成两个标量声明以维持 vec4 字段的 16 对齐。
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PanelUniform {
    pub center: [f32; 2],
    pub half_size: [f32; 2],
    pub corner_radius: f32,
    pub bevel: f32,
    pub thickness: f32,
    pub depth: f32,
    pub ior: f32,
    pub dispersion: f32,
    pub blur: f32,
    pub specular: f32,
    pub fresnel_f0: f32,
    pub velocity: [f32; 2],
    pub press: f32,
    pub tint: [f32; 4],
    pub shadow: [f32; 4],
}

impl PanelUniform {
    /// 单个面板 uniform 的大小（字节）。
    pub const SIZE: u32 = std::mem::size_of::<Self>() as u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_layout_is_96_bytes() {
        assert_eq!(std::mem::size_of::<PanelUniform>(), 96);
        assert_eq!(std::mem::size_of::<PanelUniform>() % 16, 0);
        // 关键偏移：velocity/press 占用原 _pad（52..64），vec4 字段保持 16 对齐。
        assert_eq!(std::mem::offset_of!(PanelUniform, fresnel_f0), 48);
        assert_eq!(std::mem::offset_of!(PanelUniform, velocity), 52);
        assert_eq!(std::mem::offset_of!(PanelUniform, press), 60);
        assert_eq!(std::mem::offset_of!(PanelUniform, tint), 64);
        assert_eq!(std::mem::offset_of!(PanelUniform, shadow), 80);
    }

    #[test]
    fn contains_respects_corner_radius() {
        let panel = GlassPanel::new([100.0, 100.0], [80.0, 40.0], 20.0);
        // 中心命中
        assert!(panel.contains([100.0, 100.0]));
        // 圆角外的角落不命中（SDF 把角内收了）
        assert!(!panel.contains([132.0, 116.0]));
        // 远处不命中
        assert!(!panel.contains([200.0, 200.0]));
    }

    #[test]
    fn new_starts_at_rest() {
        let panel = GlassPanel::new([10.0, 20.0], [30.0, 40.0], 8.0);
        assert_eq!(panel.velocity, [0.0, 0.0]);
        assert_eq!(panel.press, 0.0);
        assert_eq!(panel.style, GlassStyle::default());
    }
}
