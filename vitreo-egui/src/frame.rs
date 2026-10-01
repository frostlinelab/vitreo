//! egui-wgpu / egui-winit 帧接线封装。
//!
//! 把示例里逐字重复的整段接线（三件套初始化 → take input → run_ui →
//! platform output → tessellate → 无条件纹理上传 → `LoadOp::Load` 叠加 pass →
//! 提交后释放纹理）收进一个类型。纹理增量在几何为空的帧也要处理——字体
//! 图集可能到某一帧才生成而那一帧没有可绘制几何，漏处理会让字体纹理永远
//! 传不上去（见 vitreo 提交 7274329），也会让 `TexturesDelta` 在析构时
//! 因未结清而触发 epaint 的 debug 断言。

use egui::epaint;

/// egui 三件套：Context + egui-winit 状态 + egui-wgpu 渲染器。
pub struct EguiFrame {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    /// 本帧 tessellate 结果（`run_ui` 填充，`paint` 消费）。
    primitives: Vec<epaint::ClippedPrimitive>,
    /// 本帧的纹理增量（`run_ui` 填充，`paint` 消费并清空）。
    textures_delta: egui::TexturesDelta,
    /// 与 tessellation 一致的缩放系数（`run_ui` 时从 FullOutput 记录）。
    pixels_per_point: f32,
}

impl EguiFrame {
    /// 从窗口与 surface 格式构造三件套（egui 用与玻璃 pass 相同的 sRGB 目标）。
    pub fn new(
        window: &winit::window::Window,
        target_format: wgpu::TextureFormat,
        device: &wgpu::Device,
    ) -> Self {
        let ctx = egui::Context::default();
        let scale = window.scale_factor() as f32;
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(scale),
            None,
            None,
        );
        let renderer =
            egui_wgpu::Renderer::new(device, target_format, egui_wgpu::RendererOptions::default());
        Self {
            ctx,
            state,
            renderer,
            primitives: Vec::new(),
            textures_delta: egui::TexturesDelta::default(),
            pixels_per_point: scale,
        }
    }

    /// egui 上下文（安装字体、设置主题等一次性配置用）。
    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }

    /// 事件转发给 egui；返回 `consumed`——为 true 时事件已被某个控件吃掉，
    /// 不要再交给玻璃拖拽处理（否则滑块会拖动面板）。
    pub fn on_window_event(
        &mut self,
        window: &winit::window::Window,
        event: &winit::event::WindowEvent,
    ) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    /// 跑一帧 UI 逻辑：take input → `run_ui` → platform output → tessellate。
    ///
    /// 闭包收到的是全屏的 `&mut egui::Ui`（egui 0.36 的 `run_ui` 约定），
    /// 与示例的 `build_ui(ui, …)` 同形；需要 Context 时用 `ui.ctx()`。
    ///
    /// 纹理增量与图元缓存在帧内，等 [`EguiFrame::paint`] 时消费——
    /// 所以**每帧都必须先 run_ui 再 paint**，即使界面完全没变。
    pub fn run_ui(&mut self, window: &winit::window::Window, f: impl FnOnce(&mut egui::Ui)) {
        let input = self.state.take_egui_input(window);
        // egui 的 run_ui 收 FnMut；应用侧闭包只跑一次，用 Option 承接。
        let mut f = Some(f);
        let full_output = self.ctx.run_ui(input, |ui| {
            if let Some(f) = f.take() {
                f(ui);
            }
        });
        self.state.handle_platform_output(window, full_output.platform_output);
        self.pixels_per_point = full_output.pixels_per_point;
        self.primitives = self
            .ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        self.textures_delta = full_output.textures_delta;
    }

    /// 把本帧 egui 画到 `target` 上（`LoadOp::Load` 叠加，不清空玻璃内容）。
    ///
    /// 返回 egui 自己的命令缓冲——应用须把它们与自己的 encoder **一起**提交
    /// （egui 缓冲在前）：
    ///
    /// ```no_run
    /// # let mut frame: vitreo_egui::EguiFrame = unimplemented!();
    /// # let (device, queue): (&wgpu::Device, &wgpu::Queue) = unimplemented!();
    /// # let mut encoder: wgpu::CommandEncoder = unimplemented!();
    /// # let view: wgpu::TextureView = unimplemented!();
    /// let egui_cmds = frame.paint(&device, &queue, &mut encoder, &view, [1600, 900]);
    /// let mut all = egui_cmds;
    /// all.push(encoder.finish());
    /// queue.submit(all);
    /// ```
    ///
    /// 即使 `run_ui` 产生了空几何也要调用：纹理增量（字体图集）是无条件
    /// 上传的。绘制结束后立即释放本帧不再引用的纹理是安全的——wgpu 的
    /// 命令记录持有资源引用，提交完成前 GPU 侧不会被销毁。
    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size_in_pixels: [u32; 2],
    ) -> Vec<wgpu::CommandBuffer> {
        // 纹理增量必须无条件处理（不能只在有几何时处理）。
        for (id, deltas) in &self.textures_delta.set {
            for delta in deltas {
                self.renderer.update_texture(device, queue, *id, delta);
            }
        }

        let mut command_buffers = Vec::new();
        if !self.primitives.is_empty() {
            let screen_descriptor = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [size_in_pixels[0], size_in_pixels[1]],
                pixels_per_point: self.pixels_per_point,
            };
            command_buffers = self.renderer.update_buffers(
                device,
                queue,
                encoder,
                &self.primitives,
                &screen_descriptor,
            );
            {
                // forget_lifetime 消费 self，先转换再借给 render；
                // 块结束时 pass 释放对 encoder 的借用，才能 finish。
                let mut egui_pass = encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("vitreo-egui/paint"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: target,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    })
                    .forget_lifetime();
                self.renderer
                    .render(&mut egui_pass, &self.primitives, &screen_descriptor);
            }
        }

        for id in &self.textures_delta.free {
            self.renderer.free_texture(id);
        }
        self.textures_delta.clear();
        command_buffers
    }
}
