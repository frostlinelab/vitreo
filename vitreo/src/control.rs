//! 玻璃控件 —— 渲染器无关的交互原语。
//!
//! 第一个控件是按钮：胶囊玻璃体 + 悬停/按压状态 + click 判定。全部是
//! CPU 状态机叠加在弹簧/果冻管线上，不含任何 UI 框架类型——egui、iOS
//! 或任何事件源都能驱动。文字与图标是"身份"层（每平台自绘叠加），
//! 控件本身只负责玻璃体与交互语义：按下微缩（press 弹簧）、释放回弹、
//! 按钮内释放才计一次 click。

use crate::{AnimatedPanel, GlassPanel, GlassStyle, SpringConfig};

/// 玻璃按钮：胶囊玻璃体 + 指针交互 + click 判定。
///
/// ```no_run
/// # use vitreo::{GlassButton, GlassStyle, SpringConfig};
/// # let mut button = GlassButton::new(
/// #     [500.0, 400.0], [220.0, 64.0],
/// #     GlassStyle::button(), SpringConfig::smooth(),
/// # );
/// // 事件源把指针事件转给它（物理像素坐标）：
/// button.pointer_move([480.0, 390.0]); // 悬停
/// if button.pointer_down([480.0, 390.0]) {
///     // 事件被按钮吃掉，不要再交给拖拽等其他交互。
/// }
/// if button.pointer_up([480.0, 390.0]) {
///     // 一次完整的 click：按下与释放都在按钮内。
/// }
/// // 渲染：把 panel() 快照交给合成器（与其他玻璃一起画）。
/// ```
#[derive(Clone, Debug)]
pub struct GlassButton {
    panel: AnimatedPanel,
    hovered: bool,
    pressed: bool,
}

impl GlassButton {
    /// 胶囊按钮：圆角半径 = 高度的一半（经典 liquid-glass 胶囊形）。
    /// `size` 为物理像素；材质与几何弹簧独立传入。
    pub fn new(center: [f32; 2], size: [f32; 2], style: GlassStyle, config: SpringConfig) -> Self {
        let mut base = GlassPanel::new(center, size, size[1] * 0.5);
        base.style = style;
        Self {
            panel: AnimatedPanel::new(base, config),
            hovered: false,
            pressed: false,
        }
    }

    /// 当前瞬时玻璃体快照（喂给合成器）。
    pub fn panel(&self) -> GlassPanel {
        self.panel.panel()
    }

    /// 当前几何中心（物理像素）。
    pub fn center(&self) -> [f32; 2] {
        self.panel.panel().center
    }

    /// 设置中心目标（弹簧平滑移动过去）。
    pub fn set_center(&mut self, center: [f32; 2]) {
        self.panel.set_target_center(center);
    }

    /// 点（物理像素）是否命中当前几何。
    pub fn contains(&self, point: [f32; 2]) -> bool {
        self.panel.contains(point)
    }

    /// 推进全部弹簧 `dt` 秒（每帧调用一次）。
    pub fn advance(&mut self, dt: f32) {
        self.panel.advance(dt);
    }

    /// 所有弹簧是否都已收敛（静止时可用于跳帧）。
    pub fn settled(&self) -> bool {
        self.panel.settled()
    }

    /// 指针是否悬停中。
    pub fn hovered(&self) -> bool {
        self.hovered
    }

    /// 是否按压中。
    pub fn pressed(&self) -> bool {
        self.pressed
    }

    /// 指针移动：更新悬停态。
    pub fn pointer_move(&mut self, position: [f32; 2]) {
        self.hovered = self.contains(position);
    }

    /// 指针按下：命中则开始按压（微缩 + 果冻 squash）。
    /// 返回 `true` 表示事件被按钮吃掉——调用方不应再把它交给拖拽等交互。
    pub fn pointer_down(&mut self, position: [f32; 2]) -> bool {
        if !self.contains(position) {
            return false;
        }
        self.pressed = true;
        self.panel.grab();
        true
    }

    /// 指针抬起：结束按压；若在按钮内释放则完成一次 click（返回 `true`）。
    /// 未按压时调用是 no-op（返回 `false`）。
    pub fn pointer_up(&mut self, position: [f32; 2]) -> bool {
        if !self.pressed {
            return false;
        }
        self.pressed = false;
        self.panel.release();
        self.contains(position)
    }

    /// 取消（窗口失焦等）：结束按压，且不产生 click。
    pub fn cancel(&mut self) {
        if self.pressed {
            self.pressed = false;
            self.panel.release();
        }
    }

    /// 材质参数。
    pub fn style(&self) -> &GlassStyle {
        self.panel.style()
    }

    /// 材质参数（可变）。
    pub fn style_mut(&mut self) -> &mut GlassStyle {
        self.panel.style_mut()
    }

    /// DPI 变化：按比例缩放几何与像素计价材质参数。
    pub fn scale_by(&mut self, ratio: f32) {
        self.panel.scale_by(ratio);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button() -> GlassButton {
        GlassButton::new(
            [100.0, 60.0],
            [200.0, 60.0],
            GlassStyle::default(),
            SpringConfig::smooth(),
        )
    }

    fn advance_seconds(b: &mut GlassButton, seconds: f32, fps: f32) {
        let dt = 1.0 / fps;
        let steps = (seconds / dt).ceil() as u32;
        for _ in 0..steps {
            b.advance(dt);
        }
    }

    #[test]
    fn capsule_radius_is_half_height() {
        assert_eq!(button().panel().corner_radius, 30.0);
    }

    #[test]
    fn down_inside_presses_and_eats_event() {
        let mut b = button();
        assert!(b.pointer_down([150.0, 60.0]));
        assert!(b.pressed());
    }

    #[test]
    fn down_outside_is_ignored() {
        let mut b = button();
        assert!(!b.pointer_down([10.0, 10.0]));
        assert!(!b.pressed());
    }

    #[test]
    fn up_inside_after_down_completes_click() {
        let mut b = button();
        b.pointer_down([150.0, 60.0]);
        assert!(b.pointer_up([150.0, 60.0]));
        assert!(!b.pressed());
    }

    #[test]
    fn up_outside_after_down_is_no_click() {
        let mut b = button();
        b.pointer_down([150.0, 60.0]);
        assert!(!b.pointer_up([400.0, 400.0]), "按钮外释放不产生 click");
        assert!(!b.pressed());
    }

    #[test]
    fn up_without_down_is_noop() {
        let mut b = button();
        assert!(!b.pointer_up([150.0, 60.0]));
    }

    #[test]
    fn cancel_ends_press_without_click() {
        let mut b = button();
        b.pointer_down([150.0, 60.0]);
        b.cancel();
        assert!(!b.pressed());
        // 取消后的抬起不能补出一个 click。
        assert!(!b.pointer_up([150.0, 60.0]));
    }

    #[test]
    fn press_drives_micro_shrink() {
        let mut b = button();
        b.pointer_down([150.0, 60.0]);
        advance_seconds(&mut b, 0.5, 120.0);
        assert!((b.panel().press - 1.0).abs() < 0.02, "press = {}", b.panel().press);
        b.pointer_up([150.0, 60.0]);
        advance_seconds(&mut b, 1.0, 120.0);
        assert!(b.panel().press < 0.01, "释放后应回到未按压");
    }

    #[test]
    fn hover_tracks_pointer() {
        let mut b = button();
        b.pointer_move([150.0, 60.0]);
        assert!(b.hovered());
        b.pointer_move([10.0, 10.0]);
        assert!(!b.hovered());
    }

    #[test]
    fn set_center_moves_springs_to_target() {
        let mut b = button();
        b.set_center([300.0, 200.0]);
        advance_seconds(&mut b, 2.0, 60.0);
        let c = b.center();
        assert!((c[0] - 300.0).abs() < 0.5 && (c[1] - 200.0).abs() < 0.5, "center = {c:?}");
    }
}
