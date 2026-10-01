//! live-backdrop —— Vitreo 的 P2 验收 demo：
//! 应用把**自己的**动画场景渲染进离屏的 [`LiveBackdrop`]，
//! 玻璃面板折射的正是这幅每帧都在变的画面。
//!
//! 同时演示多面板重叠的两种合成策略：
//! Stack（逐层覆盖）与 Merge（并集融合成一块连续玻璃，M 键切换）。
//!
//! 帧序（同一个 encoder）：
//! 场景 pass → live.generate_mips() → compositor.render(live.backdrop()) → egui。
//!
//! ```text
//! cargo run -p live-backdrop
//! ```

use std::sync::Arc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use vitreo::{Compositor, CompositeStrategy, GlassPanel, GlassStyle, LiveBackdrop, ShadowStyle};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

const SCENE_WGSL: &str = include_str!("scene.wgsl");

/// 场景 shader 的全局量（16 字节，与 scene.wgsl 的 SceneGlobals 一致）。
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneGlobals {
    viewport: [f32; 2],
    time: f32,
    speed: f32,
}

/// 一块可拖拽玻璃面板的状态（与 minimal 相同的平滑跟随）。
struct PanelState {
    center: [f32; 2],
    target: [f32; 2],
    size: [f32; 2],
    corner_radius: f32,
    style: GlassStyle,
}

impl PanelState {
    fn panel(&self) -> GlassPanel {
        GlassPanel {
            center: self.center,
            size: self.size,
            corner_radius: self.corner_radius,
            velocity: [0.0; 2],
            press: 0.0,
            style: self.style,
        }
    }

    fn lerp_to_target(&mut self, dt: f32) {
        let k = 1.0 - (-dt * 18.0).exp();
        for i in 0..2 {
            self.center[i] += (self.target[i] - self.center[i]) * k;
        }
    }

    /// DPI 变化时按比例缩放几何与像素计价的材质参数。
    fn scale_by(&mut self, ratio: f32) {
        for i in 0..2 {
            self.size[i] *= ratio;
            self.center[i] *= ratio;
            self.target[i] *= ratio;
        }
        self.corner_radius *= ratio;
        let s = &mut self.style;
        s.bevel *= ratio;
        s.thickness *= ratio;
        s.depth *= ratio;
        s.blur *= ratio;
        s.shadow.blur *= ratio;
        s.shadow.offset = [s.shadow.offset[0] * ratio, s.shadow.offset[1] * ratio];
    }
}

fn panel_default_a(scale: f32) -> PanelState {
    PanelState {
        center: [0.0, 0.0],
        target: [0.0, 0.0],
        size: [420.0 * scale, 260.0 * scale],
        corner_radius: 64.0 * scale,
        style: GlassStyle {
            ior: 1.52,
            bevel: 40.0 * scale,
            thickness: 8.0 * scale,
            depth: 110.0 * scale,
            dispersion: 0.15,
            blur: 0.0,
            specular: 0.85,
            tint: [1.0, 1.0, 1.0],
            tint_opacity: 0.0,
            shadow: ShadowStyle {
                opacity: 0.30,
                blur: 36.0 * scale,
                offset: [0.0, 18.0 * scale],
            },
        },
    }
}

fn panel_default_b(scale: f32) -> PanelState {
    PanelState {
        center: [0.0, 0.0],
        target: [0.0, 0.0],
        size: [380.0 * scale, 240.0 * scale],
        corner_radius: 56.0 * scale,
        style: GlassStyle {
            ior: 1.60,
            bevel: 46.0 * scale,
            thickness: 9.0 * scale,
            depth: 130.0 * scale,
            dispersion: 0.22,
            blur: 0.0,
            specular: 1.0,
            tint: [0.88, 0.96, 1.0],
            tint_opacity: 0.06,
            shadow: ShadowStyle {
                opacity: 0.30,
                blur: 36.0 * scale,
                offset: [0.0, 18.0 * scale],
            },
        },
    }
}

/// egui 默认字体不含 CJK，注册平台系统中文字体（PingFang / 微软雅黑 / Noto CJK）。
fn install_cjk_font(ctx: &egui::Context) {
    let candidates = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("cjk".into(), egui::FontData::from_owned(bytes).into());
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push("cjk".into());
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
    log::warn!("未找到系统 CJK 字体，参数面板中文将显示为方块");
}

struct WindowState {
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// 离屏实时背景：场景画进 target_view，合成器采样 backdrop()。
    live: LiveBackdrop,
    compositor: Compositor,
    scene_pipeline: wgpu::RenderPipeline,
    scene_bind_group: wgpu::BindGroup,
    scene_globals: wgpu::Buffer,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    show_panel: bool,
    fps: f32,
    panels: [PanelState; 2],
    /// 正在编辑的面板（egui 单选）。
    selected: usize,
    /// 正在拖拽的面板 + 抓取偏移。
    drag: Option<(usize, [f32; 2])>,
    cursor: [f32; 2],
    strategy: CompositeStrategy,
    scene_speed: f32,
    paused: bool,
    scale_factor: f32,
}

impl WindowState {
    /// 初始摆放：两块面板中央偏下、故意部分重叠，Merge 效果开箱可见。
    fn place_panels(&mut self) {
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        let targets = [
            [0.40 * w, 0.46 * h],
            [0.58 * w, 0.55 * h],
        ];
        for (panel, target) in self.panels.iter_mut().zip(targets) {
            panel.center = target;
            panel.target = target;
        }
    }
}

struct App {
    state: Option<WindowState>,
    last_frame: Instant,
    start: Instant,
}

impl App {
    fn init_state(&mut self, window: Arc<Window>) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("no suitable GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("live-backdrop/device"),
            ..Default::default()
        }))
        .expect("request device");

        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let alpha_mode = caps
            .alpha_modes
            .iter()
            .copied()
            .find(|m| *m == wgpu::CompositeAlphaMode::Opaque)
            .unwrap_or(caps.alpha_modes[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // 离屏实时背景：sRGB 编码 + 完整 mip 链（玻璃的 blur 路径按 lod 采样）。
        let live = LiveBackdrop::new(&device, size.width.max(1), size.height.max(1));
        let compositor = Compositor::new(&device, format);

        // ---- 场景管线：画进 LiveBackdrop 的 mip-0 ----
        let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("live-backdrop/scene"),
            source: wgpu::ShaderSource::Wgsl(SCENE_WGSL.into()),
        });
        let scene_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live-backdrop/scene-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<SceneGlobals>()
                        as u64),
                },
                count: None,
            }],
        });
        let scene_globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live-backdrop/scene-globals"),
            size: std::mem::size_of::<SceneGlobals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scene_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live-backdrop/scene-bg"),
            layout: &scene_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene_globals.as_entire_binding(),
            }],
        });
        let scene_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("live-backdrop/scene-layout"),
                bind_group_layouts: &[Some(&scene_bgl)],
                immediate_size: 0,
            });
        let scene_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("live-backdrop/scene-pipeline"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &scene_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &scene_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    // LiveBackdrop 的纹理格式（Rgba8UnormSrgb）。
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let panels = [panel_default_a(scale), panel_default_b(scale)];

        let egui_ctx = egui::Context::default();
        install_cjk_font(&egui_ctx);
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(scale),
            None,
            None,
        );
        let egui_renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());

        let mut state = WindowState {
            window,
            device,
            queue,
            surface,
            config,
            live,
            compositor,
            scene_pipeline,
            scene_bind_group,
            scene_globals,
            egui_ctx,
            egui_state,
            egui_renderer,
            show_panel: true,
            fps: 0.0,
            panels,
            selected: 0,
            drag: None,
            cursor: [0.0, 0.0],
            strategy: CompositeStrategy::Merge,
            scene_speed: 1.0,
            paused: false,
            scale_factor: scale,
        };
        state.place_panels();
        self.state = Some(state);
    }

    fn render(&mut self) {
        let Some(state) = &mut self.state else {
            return;
        };
        if state.config.width == 0 || state.config.height == 0 {
            return;
        }

        let dt = self.last_frame.elapsed().as_secs_f32().min(0.1);
        self.last_frame = Instant::now();
        state.fps = state.fps * 0.9 + (1.0 / dt.max(1e-6)) * 0.1;
        for panel in &mut state.panels {
            panel.lerp_to_target(dt);
        }

        let frame = match state.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                state.surface.configure(&state.device, &state.config);
                match state.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    other => {
                        log::warn!("surface unavailable after reconfigure: {other:?}");
                        return;
                    }
                }
            }
            other => {
                // Occluded / Timeout 是正常跳帧；Validation 才是真正的错误。
                if matches!(other, wgpu::CurrentSurfaceTexture::Validation) {
                    log::error!("surface validation error: {other:?}");
                } else {
                    log::debug!("skip frame: {other:?}");
                }
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let time = self.start.elapsed().as_secs_f32();

        // ---- egui：先跑 UI 逻辑（可能修改策略/参数），再统一编码 ----
        let egui_ctx = state.egui_ctx.clone();
        let input = state.egui_state.take_egui_input(&state.window);
        let mut full_output = egui_ctx.run_ui(input, |ui| {
            let mut show = std::mem::take(&mut state.show_panel);
            egui::Panel::right("controls")
                .default_size(300.0)
                .show_collapsible(ui, &mut show, |ui| build_ui(ui, state));
            state.show_panel = show;
        });
        state
            .egui_state
            .handle_platform_output(&state.window, full_output.platform_output);
        let clipped_primitives =
            egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);

        // 纹理增量必须无条件处理：字体图集可能到某一帧才生成，而那一帧未必有
        // 可绘制的几何体。漏处理会让字体纹理永远传不上去，也会让 TexturesDelta
        // 在析构时因未结清而触发 epaint 的 debug 断言。
        for (id, deltas) in &full_output.textures_delta.set {
            for delta in deltas {
                state
                    .egui_renderer
                    .update_texture(&state.device, &state.queue, *id, delta);
            }
        }

        let pixels_per_point = state.window.scale_factor() as f32;
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [state.config.width, state.config.height],
            pixels_per_point,
        };

        let viewport = [state.config.width as f32, state.config.height as f32];
        let panels = [
            state.panels[0].panel(),
            state.panels[1].panel(),
        ];

        let mut encoder = state
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        // ---- 1. 场景 pass：应用自己的画面画进离屏实时背景 ----
        let globals = SceneGlobals {
            viewport,
            time,
            speed: if state.paused { 0.0 } else { state.scene_speed },
        };
        state
            .queue
            .write_buffer(&state.scene_globals, 0, bytemuck::bytes_of(&globals));
        {
            let mut scene_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("live-backdrop/scene-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: state.live.target_view(),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            scene_pass.set_pipeline(&state.scene_pipeline);
            scene_pass.set_bind_group(0, &state.scene_bind_group, &[]);
            scene_pass.draw(0..4, 0..1);
        }

        // ---- 2. GPU 生成 mip 链（玻璃 blur 路径按 lod 采样）----
        state.live.generate_mips(&state.device, &mut encoder);

        // ---- 3. 玻璃合成：折射的就是刚才渲染的场景 ----
        state.compositor.render(
            &state.queue,
            &mut encoder,
            state.live.backdrop(),
            &view,
            viewport,
            time,
            state.strategy,
            &panels,
        );

        // ---- 4. egui 叠加在同一帧上 ----
        if !clipped_primitives.is_empty() {
            let egui_cmds = state.egui_renderer.update_buffers(
                &state.device,
                &state.queue,
                &mut encoder,
                &clipped_primitives,
                &screen_descriptor,
            );
            {
                // forget_lifetime 消费 self，先转换再借给 render；
                // 块结束时 pass 释放对 encoder 的借用，才能 finish。
                let mut egui_pass = encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("egui"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
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
                state
                    .egui_renderer
                    .render(&mut egui_pass, &clipped_primitives, &screen_descriptor);
            }
            let mut all = egui_cmds;
            all.push(encoder.finish());
            state.queue.submit(all);
        } else {
            state.queue.submit(Some(encoder.finish()));
        }

        // 绘制结束后释放 egui 本帧不再引用的纹理。
        for id in &full_output.textures_delta.free {
            state.egui_renderer.free_texture(id);
        }
        full_output.textures_delta.clear();

        state.queue.present(frame);
    }

    fn resize(&mut self, width: u32, height: u32) {
        let Some(state) = &mut self.state else {
            return;
        };
        if width == 0 || height == 0 {
            return;
        }
        state.config.width = width;
        state.config.height = height;
        state.surface.configure(&state.device, &state.config);
        state.live.resize(&state.device, width, height);

        // 面板留在视口内
        for panel in &mut state.panels {
            for i in 0..2 {
                let lim = [width as f32, height as f32][i];
                panel.target[i] = panel.target[i].clamp(0.0, lim);
                panel.center[i] = panel.center[i].clamp(0.0, lim);
            }
        }
    }

    fn apply_scale(&mut self) {
        let Some(state) = &mut self.state else {
            return;
        };
        let new_scale = state.window.scale_factor() as f32;
        let ratio = new_scale / state.scale_factor;
        state.scale_factor = new_scale;
        // 只缩放几何与像素计价的材质参数；用户调过的物理量（ior、色散、tint）不动。
        for panel in &mut state.panels {
            panel.scale_by(ratio);
        }
    }
}

/// egui 参数面板：策略 / 场景 / 逐面板材质。
fn build_ui(ui: &mut egui::Ui, state: &mut WindowState) {
    ui.add_space(4.0);
    ui.heading("Vitreo · live-backdrop");
    ui.label(format!(
        "{:.0} fps · 玻璃折射的是本示例自己渲染的场景",
        state.fps
    ));
    ui.separator();

    ui.label(egui::RichText::new("合成策略").strong());
    let changed = ui
        .radio_value(&mut state.strategy, CompositeStrategy::Stack, "Stack 逐层覆盖")
        .changed()
        | ui
            .radio_value(&mut state.strategy, CompositeStrategy::Merge, "Merge 并集融合")
            .changed();
    if changed {
        log::info!("strategy = {:?}", state.strategy);
    }
    ui.label(
        egui::RichText::new(match state.strategy {
            CompositeStrategy::Stack => "重叠区只显示上层对背景的折射（两片分离的玻璃）",
            CompositeStrategy::Merge => "重叠面板融合成一块连续玻璃，倒角只在外轮廓",
        })
        .small()
        .weak(),
    );
    ui.label(egui::RichText::new("M 键快速切换 · 拖动玻璃面板").small().weak());
    ui.separator();

    ui.label(egui::RichText::new("场景").strong());
    ui.add(
        egui::Slider::new(&mut state.scene_speed, 0.0..=3.0)
            .text("流动速度")
            .clamping(egui::SliderClamping::Edits),
    );
    ui.checkbox(&mut state.paused, "⏸ 暂停（空格）");
    ui.separator();

    ui.label(egui::RichText::new("编辑面板").strong());
    ui.horizontal(|ui| {
        ui.radio_value(&mut state.selected, 0, "A");
        ui.radio_value(&mut state.selected, 1, "B");
    });

    let panel = &mut state.panels[state.selected];
    let style = &mut panel.style;

    egui::CollapsingHeader::new("形状")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(
                egui::Slider::new(&mut panel.size[0], 120.0..=1400.0)
                    .text("宽度 (px)")
                    .clamping(egui::SliderClamping::Edits),
            );
            ui.add(
                egui::Slider::new(&mut panel.size[1], 80.0..=900.0)
                    .text("高度 (px)")
                    .clamping(egui::SliderClamping::Edits),
            );
            let r_max = panel.size[0].min(panel.size[1]) * 0.5;
            ui.add(
                egui::Slider::new(&mut panel.corner_radius, 0.0..=r_max)
                    .text("圆角 (px)")
                    .clamping(egui::SliderClamping::Edits),
            );
        });

    egui::CollapsingHeader::new("折射")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(
                egui::Slider::new(&mut style.ior, 1.0..=2.4)
                    .text("折射率 IOR")
                    .clamping(egui::SliderClamping::Edits),
            );
            ui.add(
                egui::Slider::new(&mut style.bevel, 0.0..=140.0)
                    .text("斜面宽度")
                    .clamping(egui::SliderClamping::Edits),
            );
            ui.add(
                egui::Slider::new(&mut style.thickness, 0.0..=40.0)
                    .text("斜面厚度（法线倾角）")
                    .clamping(egui::SliderClamping::Edits),
            );
            ui.add(
                egui::Slider::new(&mut style.depth, 0.0..=320.0)
                    .text("光程（折射位移）")
                    .clamping(egui::SliderClamping::Edits),
            );
        });

    egui::CollapsingHeader::new("色散 · 高光 · 模糊")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(
                egui::Slider::new(&mut style.dispersion, 0.0..=0.5)
                    .text("色散强度（蓝红 IOR 差）")
                    .clamping(egui::SliderClamping::Edits),
            );
            if ui
                .small_button("BK7 物理色散")
                .on_hover_text("设为真实 BK7 玻璃的阿贝数色散（≈0.008，非常细微）")
                .clicked()
            {
                style.dispersion = GlassStyle::bk7_dispersion();
            }
            ui.add(
                egui::Slider::new(&mut style.specular, 0.0..=2.0)
                    .text("Fresnel 高光")
                    .clamping(egui::SliderClamping::Edits),
            );
            ui.add(
                egui::Slider::new(&mut style.blur, 0.0..=48.0)
                    .text("背景模糊（GPU mip 链）")
                    .clamping(egui::SliderClamping::Edits),
            );
        });

    egui::CollapsingHeader::new("着色").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("玻璃染色");
            let mut color = style.tint;
            ui.color_edit_button_rgb(&mut color);
            style.tint = color;
        });
        ui.add(
            egui::Slider::new(&mut style.tint_opacity, 0.0..=1.0)
                .text("染色不透明度")
                .clamping(egui::SliderClamping::Edits),
        );
    });

    egui::CollapsingHeader::new("阴影").show(ui, |ui| {
        ui.add(
            egui::Slider::new(&mut style.shadow.opacity, 0.0..=1.0)
                .text("阴影不透明度")
                .clamping(egui::SliderClamping::Edits),
        );
        ui.add(
            egui::Slider::new(&mut style.shadow.blur, 0.0..=120.0)
                .text("阴影模糊")
                .clamping(egui::SliderClamping::Edits),
        );
        ui.horizontal(|ui| {
            ui.add(
                egui::Slider::new(&mut style.shadow.offset[0], -80.0..=80.0)
                    .text("X 偏移")
                    .clamping(egui::SliderClamping::Edits),
            );
            ui.add(
                egui::Slider::new(&mut style.shadow.offset[1], -80.0..=80.0)
                    .text("Y 偏移")
                    .clamping(egui::SliderClamping::Edits),
            );
        });
    });
}

fn main() {
    env_logger::init();

    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App {
        state: None,
        last_frame: Instant::now(),
        start: Instant::now(),
    };

    event_loop.run_app(&mut app).expect("event loop run");
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_none() {
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title("Vitreo — live-backdrop（实时背景 · M 切换融合）")
                            .with_inner_size(winit::dpi::LogicalSize::new(1100.0, 680.0)),
                    )
                    .expect("create window"),
            );
            self.init_state(window);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // 快捷键必须在喂给 egui 之前处理，否则 Tab 被焦点导航消费、
        // M/Space 可能落入聚焦控件。
        if let WindowEvent::KeyboardInput { event: key, .. } = &event {
            use winit::keyboard::{Key, NamedKey};
            let toggle_panel = matches!(key.logical_key, Key::Named(NamedKey::Tab))
                || matches!(&key.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("h"));
            let toggle_merge = matches!(&key.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("m"));
            let toggle_pause = matches!(key.logical_key, Key::Named(NamedKey::Space));
            if key.state == ElementState::Pressed && !key.repeat {
                if let Some(state) = &mut self.state {
                    if toggle_panel {
                        state.show_panel = !state.show_panel;
                        return;
                    }
                    if toggle_merge {
                        state.strategy = match state.strategy {
                            CompositeStrategy::Stack => CompositeStrategy::Merge,
                            CompositeStrategy::Merge => CompositeStrategy::Stack,
                        };
                        log::info!("strategy = {:?}", state.strategy);
                        return;
                    }
                    if toggle_pause {
                        state.paused = !state.paused;
                        return;
                    }
                }
            }
        }

        // 剩余事件喂给 egui；被 UI 消费的（拖滑杆、点按钮）不再传给玻璃交互。
        if let Some(state) = &mut self.state {
            let response = state.egui_state.on_window_event(&state.window, &event);
            if response.consumed {
                return;
            }
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::Resized(size) => self.resize(size.width, size.height),
            WindowEvent::ScaleFactorChanged { .. } => self.apply_scale(),
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(state) = &mut self.state {
                    state.cursor = [position.x as f32, position.y as f32];
                    if let Some((index, grab)) = state.drag {
                        let panel = &mut state.panels[index];
                        panel.target = [state.cursor[0] - grab[0], state.cursor[1] - grab[1]];
                    }
                }
            }
            WindowEvent::MouseInput { state: button_state, button, .. } => {
                if button != MouseButton::Left {
                    return;
                }
                if let Some(ws) = &mut self.state {
                    match button_state {
                        ElementState::Pressed => {
                            // 从最上层（数组末尾）往下找命中面板。
                            let index = (0..ws.panels.len())
                                .rev()
                                .find(|&i| ws.panels[i].panel().contains(ws.cursor));
                            if let Some(i) = index {
                                ws.drag = Some((
                                    i,
                                    [
                                        ws.cursor[0] - ws.panels[i].center[0],
                                        ws.cursor[1] - ws.panels[i].center[1],
                                    ],
                                ));
                            }
                        }
                        ElementState::Released => ws.drag = None,
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }
}
