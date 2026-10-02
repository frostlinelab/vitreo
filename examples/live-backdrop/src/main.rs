//! live-backdrop —— Vitreo 的 P2 验收 demo：
//! 应用把**自己的**动画场景渲染进离屏的 [`LiveBackdrop`],
//! 玻璃面板折射的正是这幅每帧都在变的画面。
//!
//! 同时演示多面板重叠的两种合成策略：
//! Stack（逐层覆盖）与 Merge（并集融合成一块连续玻璃，M 键切换）。
//!
//! P3 起改用 `vitreo-egui`：弹簧驱动面板（拖拽带果冻回弹 + 按压微缩），
//! egui/winit/wgpu 接线全部来自绑定 crate；帧序不变：
//! 场景 pass → `GlassLayer::render_live`（内部生成 mip 链）→ egui 叠加。
//!
//! ```text
//! cargo run -p live-backdrop
//! ```

use std::sync::Arc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use vitreo::{CompositeStrategy, GlassPanel, GlassStyle, JellyStyle, ShadowStyle};
use vitreo_egui::{egui, install_cjk_fonts, EguiFrame, GlassLayer, SpringConfig};
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

fn panel_style_a(scale: f32) -> GlassStyle {
    GlassStyle {
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
        jelly: JellyStyle::default(),
    }
}

fn panel_style_b(scale: f32) -> GlassStyle {
    GlassStyle {
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
        jelly: JellyStyle::default(),
    }
}

/// build_ui 需要的可变借用集合 —— 从 WindowState 逐字段解构而来，
/// 避免闭包捕获整个 state 与 `EguiFrame::run_ui` 的借用冲突。
struct Demo<'a> {
    glass: &'a mut GlassLayer,
    fps: &'a mut f32,
    /// 正在编辑的面板（egui 单选）。
    selected: &'a mut usize,
    scene_speed: &'a mut f32,
    paused: &'a mut bool,
}

impl Demo<'_> {
    fn panel(&mut self) -> &mut vitreo_egui::AnimatedPanel {
        let index = *self.selected;
        self.glass.panel_mut(index).expect("selected panel")
    }
}

/// egui 参数面板：策略 / 场景 / 逐面板材质。
fn build_ui(ui: &mut egui::Ui, d: &mut Demo) {
    ui.add_space(4.0);
    ui.heading("Vitreo · live-backdrop");
    ui.label(format!(
        "{:.0} fps · 玻璃折射的是本示例自己渲染的场景",
        *d.fps
    ));
    ui.separator();

    ui.label(egui::RichText::new("合成策略").strong());
    let mut strategy = d.glass.strategy();
    let changed = ui
        .radio_value(&mut strategy, CompositeStrategy::Stack, "Stack 逐层覆盖")
        .changed()
        | ui
            .radio_value(&mut strategy, CompositeStrategy::Merge, "Merge 并集融合")
            .changed();
    if changed {
        d.glass.set_strategy(strategy);
        log::info!("strategy = {:?}", strategy);
    }
    ui.label(
        egui::RichText::new(match d.glass.strategy() {
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
        egui::Slider::new(d.scene_speed, 0.0..=3.0)
            .text("流动速度")
            .clamping(egui::SliderClamping::Edits),
    );
    ui.checkbox(d.paused, "⏸ 暂停（空格）");
    ui.separator();

    ui.label(egui::RichText::new("编辑面板").strong());
    ui.horizontal(|ui| {
        ui.radio_value(d.selected, 0, "A");
        ui.radio_value(d.selected, 1, "B");
    });

    egui::CollapsingHeader::new("形状")
        .default_open(true)
        .show(ui, |ui| {
            let size = d.panel().target_size();
            let mut w = size[0];
            let mut h = size[1];
            ui.add(
                egui::Slider::new(&mut w, 120.0..=1400.0)
                    .text("宽度 (px)")
                    .clamping(egui::SliderClamping::Edits),
            );
            ui.add(
                egui::Slider::new(&mut h, 80.0..=900.0)
                    .text("高度 (px)")
                    .clamping(egui::SliderClamping::Edits),
            );
            d.panel().set_target_size([w, h]);
            let r_max = size[0].min(size[1]) * 0.5;
            let mut r = d.panel().target_corner_radius();
            ui.add(
                egui::Slider::new(&mut r, 0.0..=r_max)
                    .text("圆角 (px)")
                    .clamping(egui::SliderClamping::Edits),
            );
            d.panel().set_target_corner_radius(r);
        });

    egui::CollapsingHeader::new("折射")
        .default_open(true)
        .show(ui, |ui| {
            let style = d.panel().style_mut();
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
            let style = d.panel().style_mut();
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
        let style = d.panel().style_mut();
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
        let style = d.panel().style_mut();
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

struct WindowState {
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// 弹簧玻璃层：两块面板 + 合成器 + 离屏实时背景 + 拖拽状态机。
    glass: GlassLayer,
    scene_pipeline: wgpu::RenderPipeline,
    scene_bind_group: wgpu::BindGroup,
    scene_globals: wgpu::Buffer,
    ui: EguiFrame,
    show_panel: bool,
    fps: f32,
    /// 正在编辑的面板（egui 单选）。
    selected: usize,
    scene_speed: f32,
    paused: bool,
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

        // 弹簧玻璃层：离屏实时背景（Rgba8UnormSrgb，与 surface 格式无关）。
        // 两块面板中央偏下、故意部分重叠，Merge 效果开箱可见。
        let mut glass = GlassLayer::with_live_backdrop(
            &device,
            format,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            scale,
            size.width.max(1),
            size.height.max(1),
        );
        let (w, h) = (size.width as f32, size.height as f32);
        let mut panel_a = GlassPanel::new([0.40 * w, 0.46 * h], [420.0 * scale, 260.0 * scale], 64.0 * scale);
        panel_a.style = panel_style_a(scale);
        let mut panel_b = GlassPanel::new([0.58 * w, 0.55 * h], [380.0 * scale, 240.0 * scale], 56.0 * scale);
        panel_b.style = panel_style_b(scale);
        glass
            .add_panel(panel_a, SpringConfig::bouncy())
            .expect("panel capacity");
        glass
            .add_panel(panel_b, SpringConfig::bouncy())
            .expect("panel capacity");
        glass.set_strategy(CompositeStrategy::Merge);

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
                    // GlassLayer 离屏实时背景的纹理格式（Rgba8UnormSrgb）。
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

        let ui = EguiFrame::new(&window, format, &device);
        install_cjk_fonts(ui.ctx());

        self.state = Some(WindowState {
            window,
            device,
            queue,
            surface,
            config,
            glass,
            scene_pipeline,
            scene_bind_group,
            scene_globals,
            ui,
            show_panel: true,
            fps: 0.0,
            selected: 0,
            scene_speed: 1.0,
            paused: false,
        });
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
        state.glass.advance(dt);

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

        // ---- UI 阶段：解构字段得到互不相交的借用 ----
        let WindowState {
            window,
            device,
            queue,
            surface: _,
            config,
            glass,
            scene_pipeline,
            scene_bind_group,
            scene_globals,
            ui,
            show_panel,
            fps,
            selected,
            scene_speed,
            paused,
        } = state;

        let mut demo = Demo {
            glass,
            fps,
            selected,
            scene_speed,
            paused,
        };
        ui.run_ui(window, |ui| {
            let mut show = std::mem::take(show_panel);
            egui::Panel::right("controls")
                .default_size(300.0)
                .show_collapsible(ui, &mut show, |ui| build_ui(ui, &mut demo));
            *show_panel = show;
        });

        let viewport = [config.width as f32, config.height as f32];
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        // ---- 1. 场景 pass：应用自己的画面画进离屏实时背景 ----
        let globals = SceneGlobals {
            viewport,
            time,
            speed: if *paused { 0.0 } else { *scene_speed },
        };
        queue.write_buffer(scene_globals, 0, bytemuck::bytes_of(&globals));
        {
            let scene_target = glass.scene_target_view().expect("live backdrop target");
            let mut scene_pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("live-backdrop/scene-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: scene_target,
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
                })
                .forget_lifetime();
            scene_pass.set_pipeline(scene_pipeline);
            scene_pass.set_bind_group(0, &*scene_bind_group, &[]);
            scene_pass.draw(0..4, 0..1);
        }

        // ---- 2+3. 玻璃合成：render_live 内部生成 mip 链（blur 路径按 lod 采样），
        //      玻璃折射的正是刚才渲染的场景 ----
        glass.render_live(device, queue, &mut encoder, &view, time);

        // ---- 4. egui 叠加 + 合并提交 ----
        let egui_cmds = ui.paint(device, queue, &mut encoder, &view, [config.width, config.height]);
        let mut all = egui_cmds;
        all.push(encoder.finish());
        queue.submit(all);

        queue.present(frame);
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

        // 重建离屏背景 + 面板留在视口内。
        state.glass.resize(&state.device, width, height);
    }

    fn apply_scale(&mut self) {
        let Some(state) = &mut self.state else {
            return;
        };
        // 几何与像素计价材质按比例缩放；用户调过的物理量（ior、色散、tint）不动。
        let scale = state.window.scale_factor() as f32;
        state.glass.apply_scale_factor(scale);
    }
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
                        let strategy = match state.glass.strategy() {
                            CompositeStrategy::Stack => CompositeStrategy::Merge,
                            CompositeStrategy::Merge => CompositeStrategy::Stack,
                        };
                        state.glass.set_strategy(strategy);
                        log::info!("strategy = {:?}", strategy);
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
            if state.ui.on_window_event(&state.window, &event) {
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
                    state
                        .glass
                        .pointer_moved([position.x as f32, position.y as f32]);
                }
            }
            WindowEvent::MouseInput { state: button_state, button, .. } => {
                if button != MouseButton::Left {
                    return;
                }
                if let Some(ws) = &mut self.state {
                    match button_state {
                        // 从最上层往下命中（GlassLayer 内部处理），命中即抓取。
                        ElementState::Pressed => {
                            ws.glass.pointer_press();
                        }
                        ElementState::Released => {
                            ws.glass.pointer_release();
                        }
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
