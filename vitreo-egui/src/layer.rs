//! 多面板玻璃层 —— [`Compositor`] + 弹簧面板 + 拖拽状态机 + 可选离屏实时背景。
//!
//! 帧序中玻璃层的职责（见 crate 文档）：
//! 1. UI 逻辑先行（它可能改面板参数/策略）；
//! 2. [`GlassLayer::advance`] 推进弹簧；
//! 3. [`GlassLayer::render_static`]（静态背景）或 [`GlassLayer::render_live`]
//!    （应用自身场景折射）完成合成。
//!
//! 拖拽状态机（[`DragState`])是纯 CPU 逻辑，独立测试；`GlassLayer` 自身
//! 持有 wgpu 资源，无法在无 GPU 环境下构造。

use vitreo::{Backdrop, CompositeStrategy, Compositor, GlassPanel, LiveBackdrop};
use vitreo::compositor::MAX_PANELS;

use crate::animated::AnimatedPanel;
use crate::spring::SpringConfig;

/// 指针拖拽状态机：命中检测（取最上层）→ 记录抓取偏移 → 移动跟随 → 释放。
#[derive(Clone, Copy, Debug, Default)]
struct DragState {
    cursor: [f32; 2],
    /// (面板下标, 抓取偏移 = 光标 − 面板中心)。
    grab: Option<(usize, [f32; 2])>,
}

impl DragState {
    /// 按下：命中测试，命中则抓取并返回面板下标。
    fn press(&mut self, panels: &[AnimatedPanel], position: [f32; 2]) -> Option<usize> {
        self.cursor = position;
        let index = panels.iter().rposition(|p| p.contains(position))?;
        let center = panels[index].panel().center;
        self.grab = Some((index, [position[0] - center[0], position[1] - center[1]]));
        Some(index)
    }

    /// 移动：抓取中则让目标点跟随光标（保持偏移）。
    fn moved(&mut self, panels: &mut [AnimatedPanel], position: [f32; 2]) {
        self.cursor = position;
        if let Some((index, offset)) = self.grab {
            panels[index].set_target_center([
                position[0] - offset[0],
                position[1] - offset[1],
            ]);
        }
    }

    /// 释放：结束抓取并返回面板下标。
    fn release(&mut self, panels: &mut [AnimatedPanel]) -> Option<usize> {
        let (index, _) = self.grab.take()?;
        panels[index].release();
        Some(index)
    }
}

/// 玻璃层：至多 [`MAX_PANELS`] 块弹簧面板 + 合成策略 + 可选的
/// [`LiveBackdrop`] 离屏管线。
pub struct GlassLayer {
    compositor: Compositor,
    panels: Vec<AnimatedPanel>,
    strategy: CompositeStrategy,
    live: Option<LiveBackdrop>,
    scale_factor: f32,
    drag: DragState,
}

impl GlassLayer {
    /// 静态背景模式（`Backdrop::from_rgba` 的图片/渐变背景）。
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat, scale_factor: f32) -> Self {
        Self {
            compositor: Compositor::new(device, target_format),
            panels: Vec::new(),
            strategy: CompositeStrategy::default(),
            live: None,
            scale_factor,
            drag: DragState::default(),
        }
    }

    /// 离屏实时背景模式：应用把自身场景画进 [`GlassLayer::scene_target_view`]，
    /// 玻璃折射的正是这幅每帧更新的画面。
    ///
    /// 两个格式各司其职：`target_format` 是合成目标（surface）的格式，
    /// `backdrop_format` 是离屏场景纹理的格式（须为 sRGB，
    /// `LiveBackdrop::with_format` 会断言）——离屏纹理不必与 surface 同格式。
    pub fn with_live_backdrop(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        backdrop_format: wgpu::TextureFormat,
        scale_factor: f32,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            live: Some(LiveBackdrop::with_format(device, width, height, backdrop_format)),
            ..Self::new(device, target_format, scale_factor)
        }
    }

    /// 添加面板（弹簧配置决定回弹手感）。满员时返回 `None`。
    pub fn add_panel(&mut self, panel: GlassPanel, config: SpringConfig) -> Option<usize> {
        if self.panels.len() >= MAX_PANELS {
            return None;
        }
        self.panels.push(AnimatedPanel::new(panel, config));
        Some(self.panels.len() - 1)
    }

    /// 面板数量。
    pub fn len(&self) -> usize {
        self.panels.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.panels.is_empty()
    }

    /// 面板切片（数组顺序即 Stack 的层次顺序，末尾最上层）。
    pub fn panels(&self) -> &[AnimatedPanel] {
        &self.panels
    }

    /// 按下标的可变面板。
    pub fn panel_mut(&mut self, index: usize) -> Option<&mut AnimatedPanel> {
        self.panels.get_mut(index)
    }

    /// 点位命中的最上层面板（无则 `None`）。
    pub fn topmost_at(&self, position: [f32; 2]) -> Option<usize> {
        self.panels.iter().rposition(|p| p.contains(position))
    }

    /// 合成策略。
    pub fn strategy(&self) -> CompositeStrategy {
        self.strategy
    }

    /// 切换合成策略（Stack / Merge）。
    pub fn set_strategy(&mut self, strategy: CompositeStrategy) {
        self.strategy = strategy;
    }

    /// 当前 DPI 缩放系数。
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// DPI 变化：所有面板按比例缩放几何与像素计价材质参数。
    pub fn apply_scale_factor(&mut self, new_scale: f32) {
        let ratio = new_scale / self.scale_factor;
        for panel in &mut self.panels {
            panel.scale_by(ratio);
        }
        self.scale_factor = new_scale;
    }

    /// 推进全部面板弹簧 `dt` 秒（每帧调用一次）。
    pub fn advance(&mut self, dt: f32) {
        for panel in &mut self.panels {
            panel.advance(dt);
        }
    }

    /// 是否所有面板都已收敛（可用于静止跳帧优化）。
    pub fn settled(&self) -> bool {
        self.panels.iter().all(|p| p.settled())
    }

    // ---------- 指针事件（物理像素） ----------

    /// 移动：抓取中则拖动目标点；同时记录光标位置供 [`GlassLayer::pointer_press`]
    /// 的命中测试使用。
    pub fn pointer_moved(&mut self, position: [f32; 2]) {
        self.drag.moved(&mut self.panels, position);
    }

    /// 按下：在最近一次 [`GlassLayer::pointer_moved`] 的位置做命中测试，
    /// 命中则抓取最上层面板并返回其下标。
    pub fn pointer_press(&mut self) -> Option<usize> {
        let position = self.drag.cursor;
        let index = self.drag.press(&self.panels, position)?;
        self.panels[index].grab();
        Some(index)
    }

    /// 释放：结束拖拽，返回被释放的面板下标。
    pub fn pointer_release(&mut self) -> Option<usize> {
        self.drag.release(&mut self.panels)
    }

    /// 正在被抓取的面板下标。
    pub fn dragging(&self) -> Option<usize> {
        self.drag.grab.map(|(index, _)| index)
    }

    /// resize：把面板中心钳制进新视口，并同步重建离屏实时背景。
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if let Some(live) = &mut self.live {
            live.resize(device, width, height);
        }
        self.clamp_panels_to([width as f32, height as f32]);
    }

    /// 只钳制面板中心进视口（静态背景模式下 resize 用）。
    pub fn clamp_panels_to(&mut self, viewport: [f32; 2]) {
        for panel in &mut self.panels {
            panel.clamp_center_to(viewport);
        }
    }

    // ---------- 渲染 ----------

    /// 实时背景模式下的场景渲染目标（每帧先把它画满）。
    pub fn scene_target_view(&self) -> Option<&wgpu::TextureView> {
        self.live.as_ref().map(|live| live.target_view())
    }

    /// 静态背景合成：`backdrop` 须覆盖整个视口（`viewport` 为物理像素）。
    pub fn render_static(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        backdrop: &Backdrop,
        target: &wgpu::TextureView,
        viewport: [f32; 2],
        time: f32,
    ) {
        let (buf, count) = self.snapshot();
        self.compositor.render(
            queue,
            encoder,
            backdrop,
            target,
            viewport,
            time,
            self.strategy,
            &buf[..count],
        );
    }

    /// 实时背景合成：内部先 GPU 生成 mip 链（blur 路径按 lod 采样），
    /// 再用 `live.size()` 作为视口合成。
    pub fn render_live(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        time: f32,
    ) {
        let live = self
            .live
            .as_ref()
            .expect("render_live 需要 with_live_backdrop 构造的 GlassLayer");
        live.generate_mips(device, encoder);
        let size = live.size();
        let (buf, count) = self.snapshot();
        self.compositor.render(
            queue,
            encoder,
            live.backdrop(),
            target,
            [size[0] as f32, size[1] as f32],
            time,
            self.strategy,
            &buf[..count],
        );
    }

    /// 面板快照：栈上定长缓冲 + 有效数量（避免每帧堆分配）。
    fn snapshot(&self) -> ([GlassPanel; MAX_PANELS], usize) {
        let mut buf = [GlassPanel::new([0.0; 2], [0.0; 2], 0.0); MAX_PANELS];
        for (slot, panel) in buf.iter_mut().zip(&self.panels) {
            *slot = panel.panel();
        }
        (buf, self.panels.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spring::SpringConfig;
    use vitreo::GlassPanel;

    /// 拖拽状态机可以在无 GPU 环境下用面板切片直接驱动。
    fn panels() -> Vec<AnimatedPanel> {
        vec![
            AnimatedPanel::new(GlassPanel::new([300.0, 300.0], [400.0, 200.0], 40.0), SpringConfig::smooth()),
            AnimatedPanel::new(GlassPanel::new([400.0, 300.0], [400.0, 200.0], 40.0), SpringConfig::smooth()),
        ]
    }

    #[test]
    fn press_hits_topmost_at_overlap() {
        // 两块面板在 x ∈ [200, 600] 重叠；重叠区命中数组末尾的那块（下标 1，
        // Stack 顺序里最上层）。抓取时调用 panel.grab() 是 GlassLayer 的职责。
        let panels = panels();
        let mut drag = DragState::default();
        let hit = drag.press(&panels, [400.0, 300.0]);
        assert_eq!(hit, Some(1));
        let hit_outside = drag.press(&panels, [10.0, 10.0]);
        assert_eq!(hit_outside, None);
    }

    #[test]
    fn drag_moves_target_preserving_grab_offset() {
        let mut panels = panels();
        let mut drag = DragState::default();
        // 按在 (410, 310)——面板 1 中心 (400,300) 右下 10px 处。
        drag.press(&panels, [410.0, 310.0]);
        drag.moved(&mut panels, [500.0, 320.0]);
        // 目标 = 光标 − 偏移 = (500-10, 320-10)。
        assert_eq!(panels[1].target_center(), [490.0, 310.0]);
        // 面板 0 不受影响。
        assert_eq!(panels[0].target_center(), [300.0, 300.0]);
    }

    #[test]
    fn release_ends_drag_and_clears_grab() {
        let mut panels = panels();
        let mut drag = DragState::default();
        let grabbed = drag.press(&panels, [400.0, 300.0]);
        assert_eq!(grabbed, Some(1));
        let released = drag.release(&mut panels);
        assert_eq!(released, Some(1));
        assert_eq!(drag.release(&mut panels), None, "二次释放应为空");
        // 按压回落是动画层的事（animated::release 测试覆盖），这里只验证
        // 抓取状态被清空、后续 move 不再拖动任何面板。
        drag.moved(&mut panels, [900.0, 900.0]);
        assert_eq!(panels[1].target_center(), [400.0, 300.0]);
    }

    #[test]
    fn drag_press_then_move_without_hit_is_noop() {
        let mut panels = panels();
        let mut drag = DragState::default();
        drag.press(&panels, [10.0, 10.0]); // 未命中
        drag.moved(&mut panels, [500.0, 500.0]);
        for (i, p) in panels.iter().enumerate() {
            assert_eq!(p.target_center(), if i == 0 { [300.0, 300.0] } else { [400.0, 300.0] });
        }
    }
}
