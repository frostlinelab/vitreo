//! 弹簧驱动的玻璃面板 —— [`GlassPanel`] 的动画外壳。
//!
//! 中心、尺寸、圆角各挂弹簧；位置弹簧的速度直接喂给
//! `GlassPanel::velocity`，着色器端由此产生果冻形变（squash & stretch +
//! 法线滞后），按压弹簧驱动整体微缩。数学本体在 `vitreo::sdf2d::jelly_*`。

use vitreo::{GlassPanel, GlassStyle};

use crate::spring::{Spring, SpringConfig};

/// 一块可动画玻璃面板：弹簧状态 + 材质。
#[derive(Clone, Debug)]
pub struct AnimatedPanel {
    cx: Spring,
    cy: Spring,
    w: Spring,
    h: Spring,
    radius: Spring,
    press: Spring,
    style: GlassStyle,
}

impl AnimatedPanel {
    /// 从静态面板出发（弹簧瞬移到该几何，继承其速度）。
    pub fn new(panel: GlassPanel, config: SpringConfig) -> Self {
        Self {
            cx: Spring::with_velocity(panel.center[0], panel.velocity[0], config),
            cy: Spring::with_velocity(panel.center[1], panel.velocity[1], config),
            w: Spring::new(panel.size[0], config),
            h: Spring::new(panel.size[1], config),
            radius: Spring::new(panel.corner_radius, config),
            press: Spring::new(panel.press, config),
            style: panel.style,
        }
    }

    /// 中心目标（拖拽目标点）。
    pub fn target_center(&self) -> [f32; 2] {
        [self.cx.target(), self.cy.target()]
    }

    /// 设置中心目标。速度保持——果冻感来自这里。
    pub fn set_target_center(&mut self, center: [f32; 2]) {
        self.cx.set_target(center[0]);
        self.cy.set_target(center[1]);
    }

    /// 设置尺寸目标。
    pub fn set_target_size(&mut self, size: [f32; 2]) {
        self.w.set_target(size[0]);
        self.h.set_target(size[1]);
    }

    /// 当前尺寸目标（UI 滑杆读写用）。
    pub fn target_size(&self) -> [f32; 2] {
        [self.w.target(), self.h.target()]
    }

    /// 设置圆角半径目标。
    pub fn set_target_corner_radius(&mut self, radius: f32) {
        self.radius.set_target(radius);
    }

    /// 当前圆角半径目标。
    pub fn target_corner_radius(&self) -> f32 {
        self.radius.target()
    }

    /// 把中心（当前值与目标）钳制进视口——resize 时防止面板漂出屏幕。
    pub fn clamp_center_to(&mut self, viewport: [f32; 2]) {
        for (spring, limit) in [(&mut self.cx, viewport[0]), (&mut self.cy, viewport[1])] {
            let clamped = spring.value().clamp(0.0, limit);
            let target = spring.target().clamp(0.0, limit);
            spring.jump_to(clamped);
            spring.set_target(target);
        }
    }

    /// 抓取：按压弹簧抬起（着色器端整体微缩）。
    pub fn grab(&mut self) {
        self.press.set_target(1.0);
    }

    /// 释放。
    pub fn release(&mut self) {
        self.press.set_target(0.0);
    }

    /// 推进所有弹簧 `dt` 秒。
    pub fn advance(&mut self, dt: f32) {
        self.cx.advance(dt);
        self.cy.advance(dt);
        self.w.advance(dt);
        self.h.advance(dt);
        self.radius.advance(dt);
        self.press.advance(dt);
    }

    /// 当前瞬时面板：几何取弹簧当前值，`velocity` 取位置弹簧速度（px/s），
    /// `press` 取按压弹簧值。
    pub fn panel(&self) -> GlassPanel {
        GlassPanel {
            center: [self.cx.value(), self.cy.value()],
            size: [self.w.value(), self.h.value()],
            corner_radius: self.radius.value(),
            velocity: [self.cx.velocity(), self.cy.velocity()],
            press: self.press.value(),
            style: self.style,
        }
    }

    /// 点（物理像素）是否命中当前几何。
    pub fn contains(&self, point: [f32; 2]) -> bool {
        self.panel().contains(point)
    }

    /// 材质参数。
    pub fn style(&self) -> &GlassStyle {
        &self.style
    }

    /// 材质参数（可变）。
    pub fn style_mut(&mut self) -> &mut GlassStyle {
        &mut self.style
    }

    /// 所有弹簧是否都已收敛。
    pub fn settled(&self) -> bool {
        self.cx.settled()
            && self.cy.settled()
            && self.w.settled()
            && self.h.settled()
            && self.radius.settled()
            && self.press.settled()
    }

    /// DPI 变化：按比例缩放几何（弹簧值/目标/速度）与**像素计价**的材质参数
    /// （bevel、thickness、depth、blur、shadow）。用户调过的物理/外观参数
    /// （ior、dispersion、tint、specular、tint_opacity）保持不动。
    pub fn scale_by(&mut self, ratio: f32) {
        for spring in [&mut self.cx, &mut self.cy, &mut self.w, &mut self.h, &mut self.radius] {
            spring.scale(ratio);
        }
        let s = &mut self.style;
        s.bevel *= ratio;
        s.thickness *= ratio;
        s.depth *= ratio;
        s.blur *= ratio;
        s.shadow.blur *= ratio;
        s.shadow.offset = [s.shadow.offset[0] * ratio, s.shadow.offset[1] * ratio];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vitreo::ShadowStyle;

    fn panel_at(center: [f32; 2]) -> AnimatedPanel {
        let mut p = GlassPanel::new(center, [420.0, 260.0], 64.0);
        p.style.bevel = 40.0;
        p.style.depth = 110.0;
        p.style.shadow = ShadowStyle { opacity: 0.3, blur: 36.0, offset: [0.0, 18.0] };
        AnimatedPanel::new(p, SpringConfig::smooth())
    }

    #[test]
    fn springs_carry_velocity_into_panel() {
        let mut p = panel_at([0.0, 0.0]);
        p.set_target_center([400.0, 0.0]);
        p.advance(1.0 / 120.0);
        let panel = p.panel();
        assert!(panel.velocity[0] > 0.0, "释放瞬间应有正向速度");
        assert_eq!(panel.velocity[1], 0.0);
    }

    #[test]
    fn press_ramps_with_grab_and_release() {
        let mut p = panel_at([0.0, 0.0]);
        p.grab();
        for _ in 0..60 {
            p.advance(1.0 / 120.0);
        }
        assert!((p.panel().press - 1.0).abs() < 0.02, "press = {}", p.panel().press);
        p.release();
        for _ in 0..120 {
            p.advance(1.0 / 120.0);
        }
        assert!(p.panel().press < 0.01);
    }

    #[test]
    fn size_and_radius_follow_targets() {
        let mut p = panel_at([0.0, 0.0]);
        p.set_target_size([300.0, 200.0]);
        p.set_target_corner_radius(32.0);
        for _ in 0..240 {
            p.advance(1.0 / 120.0);
        }
        let panel = p.panel();
        assert!((panel.size[0] - 300.0).abs() < 0.5);
        assert!((panel.size[1] - 200.0).abs() < 0.5);
        assert!((panel.corner_radius - 32.0).abs() < 0.5);
        assert!(p.settled());
    }

    #[test]
    fn scale_by_scales_geometry_and_pixel_priced_style_only() {
        let mut p = panel_at([100.0, 60.0]);
        p.style.ior = 1.7;
        p.style.dispersion = 0.2;
        p.style.tint = [0.5, 0.6, 0.7];
        p.style.specular = 1.2;
        p.style.tint_opacity = 0.3;

        p.scale_by(2.0);
        let panel = p.panel();
        assert_eq!(panel.center, [200.0, 120.0]);
        assert_eq!(panel.size, [840.0, 520.0]);
        assert_eq!(panel.corner_radius, 128.0);
        let s = p.style();
        assert_eq!(s.bevel, 80.0);
        assert_eq!(s.depth, 220.0);
        assert_eq!(s.shadow.blur, 72.0);
        assert_eq!(s.shadow.offset, [0.0, 36.0]);
        // 物理/外观参数不动
        assert_eq!(s.ior, 1.7);
        assert_eq!(s.dispersion, 0.2);
        assert_eq!(s.tint, [0.5, 0.6, 0.7]);
        assert_eq!(s.specular, 1.2);
        assert_eq!(s.tint_opacity, 0.3);
    }

    #[test]
    fn clamp_pulls_center_back_into_viewport() {
        let mut p = panel_at([1800.0, 1200.0]);
        p.clamp_center_to([1600.0, 900.0]);
        let panel = p.panel();
        assert_eq!(panel.center, [1600.0, 900.0]);
        assert_eq!(p.target_center(), [1600.0, 900.0]);
    }

    #[test]
    fn hit_test_uses_current_geometry() {
        let p = panel_at([500.0, 300.0]);
        assert!(p.contains([500.0, 300.0]));
        assert!(p.contains([500.0 + 200.0, 300.0]));
        assert!(!p.contains([500.0 + 400.0, 300.0]));
    }
}
