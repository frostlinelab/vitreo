//! gallery —— Vitreo 控件展览：所有玻璃控件的展示厅。
//!
//! 当前展出的唯一控件是 GlassButton：6 颗不同材质家族的胶囊按钮，
//! 每颗独立处理悬停（specular 提亮）/ 按压（微缩 + 果冻）/ 点击计数。
//! 按钮文字是"身份"层（由平台 egui 画在玻璃体之上），玻璃体与交互
//! 语义全部来自 `vitreo` 核心——任何事件源都能驱动同样的控件。
//! 右侧导览面板介绍每个展品；Tab 键切换显隐。
//!
//! 用法：
//! ```text
//! cargo run -p gallery
//! ```

mod backdrop_gen;

use std::sync::Arc;
use std::time::Instant;

use vitreo::{Backdrop, GlassButton, GlassLayer, GlassPanel, GlassStyle, SpringConfig};
use vitreo_egui::{egui, install_cjk_fonts, EguiFrame};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

/// 展品总数（2 列 × 3 行，须 ≤ 合成器上限 8）。
const EXHIBIT_COUNT: usize = 6;

/// 一件展品：控件玻璃体 + 身份层文字 + 点击计数。
/// 未来控件（滑块等）作为新展品类型加入展览——渲染与事件路由只依赖
/// 这个最小接口，不关心具体是哪种控件。
struct Exhibit {
    /// 展品名（玻璃体上的标签 + 导览面板标题）。
    name: &'static str,
    /// 一句话说明（导览面板展示）。
    desc: &'static str,
    button: GlassButton,
    /// 点击计数（导览面板展示）。
    clicks: u32,
    /// 未悬停时的 specular；悬停时 +0.25 提亮。
    base_specular: f32,
}

impl Exhibit {
    fn new(
        name: &'static str,
        desc: &'static str,
        center: [f32; 2],
        size: [f32; 2],
        style: GlassStyle,
        config: SpringConfig,
    ) -> Self {
        Self {
            name,
            desc,
            base_specular: style.specular,
            button: GlassButton::new(center, size, style, config),
            clicks: 0,
        }
    }
}

/// 展品材质：以 `GlassStyle::button()` 为底，像素计价字段按 DPI 缩放，
/// `adjust` 再按材质家族微调。
fn exhibit_style(scale: f32, adjust: impl FnOnce(&mut GlassStyle)) -> GlassStyle {
    let mut style = GlassStyle::button();
    style.bevel *= scale;
    style.thickness *= scale;
    style.depth *= scale;
    style.blur *= scale;
    style.shadow.blur *= scale;
    style.shadow.offset[0] *= scale;
    style.shadow.offset[1] *= scale;
    adjust(&mut style);
    style
}

/// 展品位置：2 列 × 3 行，按视口比例计算（resize 时弹簧归位）。
fn exhibit_slots(viewport: [f32; 2]) -> [[f32; 2]; EXHIBIT_COUNT] {
    let col_x = [0.30, 0.70];
    let row_y = [0.25, 0.50, 0.75];
    let mut slots = [[0.0; 2]; EXHIBIT_COUNT];
    for (i, slot) in slots.iter_mut().enumerate() {
        *slot = [viewport[0] * col_x[i % 2], viewport[1] * row_y[i / 2]];
    }
    slots
}

/// 6 件展品：材质家族覆盖默认 / 水晶 / 磨砂 / 高色散 / 着色 / 紧凑+bouncy。
fn build_exhibits(viewport: [f32; 2], scale: f32) -> Vec<Exhibit> {
    let slots = exhibit_slots(viewport);
    let size = [240.0 * scale, 64.0 * scale];

    let mut exhibits = Vec::new();
    exhibits.push(Exhibit::new(
        "默认",
        "GlassStyle::button() 原版胶囊按钮",
        slots[0],
        size,
        exhibit_style(scale, |_| {}),
        SpringConfig::smooth(),
    ));
    exhibits.push(Exhibit::new(
        "水晶",
        "高 IOR 水晶玻璃：清晰透亮",
        slots[1],
        size,
        exhibit_style(scale, |s| {
            s.ior = 1.62;
            s.bevel = 34.0 * scale;
            s.thickness = 12.0 * scale;
            s.depth = 150.0 * scale;
            s.dispersion = 0.10;
            s.specular = 1.25;
            s.tint = [0.94, 0.98, 1.0];
            s.tint_opacity = 0.08;
        }),
        SpringConfig::smooth(),
    ));
    exhibits.push(Exhibit::new(
        "磨砂",
        "低 IOR + 背景模糊：雾面质感",
        slots[2],
        size,
        exhibit_style(scale, |s| {
            s.ior = 1.45;
            s.depth = 70.0 * scale;
            s.dispersion = 0.03;
            s.blur = 14.0 * scale;
            s.specular = 0.80;
            s.tint_opacity = 0.14;
        }),
        SpringConfig::smooth(),
    ));
    exhibits.push(Exhibit::new(
        "彩虹棱镜",
        "高色散：折射边缘彩虹分离",
        slots[3],
        size,
        exhibit_style(scale, |s| {
            s.ior = 1.75;
            s.bevel = 40.0 * scale;
            s.depth = 180.0 * scale;
            s.dispersion = 0.50;
            s.blur = 0.0;
            s.specular = 1.0;
        }),
        SpringConfig::smooth(),
    ));
    exhibits.push(Exhibit::new(
        "着色",
        "蓝色染色玻璃",
        slots[4],
        size,
        exhibit_style(scale, |s| {
            s.tint = [0.55, 0.75, 1.0];
            s.tint_opacity = 0.22;
        }),
        SpringConfig::smooth(),
    ));
    exhibits.push(Exhibit::new(
        "紧凑",
        "小尺寸 + bouncy 弹簧：按压回弹更弹",
        slots[5],
        [150.0 * scale, 46.0 * scale],
        exhibit_style(scale, |_| {}),
        SpringConfig::bouncy(),
    ));
    exhibits
}

/// build_ui 需要的可变借用集合 —— 从 WindowState 逐字段解构而来，
/// 避免闭包捕获整个 state 与 `EguiFrame::run_ui` 的借用冲突。
struct Guide<'a> {
    exhibits: &'a mut Vec<Exhibit>,
    fps: &'a mut f32,
}

/// egui 导览面板：每个展品的名称 / 点击计数 / 一句话说明 + 重置计数。
/// 面板本体（`Panel::right + show_collapsible`）在 render 循环里创建，
/// 这里只画内容。
fn build_ui(ui: &mut egui::Ui, g: &mut Guide) {
    ui.add_space(4.0);
    ui.heading("控件展览");
    ui.label(format!(
        "Vitreo GlassButton · {:.0} fps · Tab 隐藏导览",
        *g.fps
    ));
    ui.label(
        egui::RichText::new("悬停提亮 · 按压微缩 · 按钮内释放计一次点击")
            .small()
            .weak(),
    );
    ui.separator();

    for e in g.exhibits.iter() {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(e.name).strong());
            ui.label(egui::RichText::new(format!("点击 {}", e.clicks)).weak());
        });
        ui.label(egui::RichText::new(e.desc).small().weak());
        ui.add_space(2.0);
    }

    ui.separator();
    if ui.button("重置计数").clicked() {
        for e in g.exhibits.iter_mut() {
            e.clicks = 0;
        }
    }
}

struct WindowState {
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    backdrop: Backdrop,
    /// 玻璃层：本展览没有自由面板，只借用它的控件合成路径
    /// （`render_static_with_controls`，面板数 + 控件数 ≤ 8）。
    glass: GlassLayer,
    /// 展品阵列（当前全部是 GlassButton）。
    exhibits: Vec<Exhibit>,
    /// 展品创建时的 DPI（缩放换算基准）。
    scale_base: f32,
    /// 最近一次光标位置（物理像素，按钮命中测试用）。
    cursor: [f32; 2],
    /// egui 三件套 + 帧接线。
    ui: EguiFrame,
    show_panel: bool,
    fps: f32,
}

struct App {
    state: Option<WindowState>,
    last_frame: Instant,
    start: Instant,
    first_frame_done: bool,
}

/// 启动闪屏帧：surface 配置好就立刻呈现背景主题色。
/// 之后的背景生成、（冷 Metal 缓存下的）管线编译、字体装载都发生在
/// 这帧之后——窗口从出现的第一刻起就是主题色，而不是系统白窗。
fn present_splash(device: &wgpu::Device, queue: &wgpu::Queue, surface: &wgpu::Surface<'_>) {
    let frame = match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame)
        | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
        _ => return,
    };
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("gallery/splash"),
    });
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("gallery/splash-pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: wgpu::Operations {
                // 背景渐变的起点色（线性空间），与 backdrop_gen 的首个停靠点一致。
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: 0.075,
                    g: 0.043,
                    b: 0.212,
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Store,
            },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    queue.submit([encoder.finish()]);
    queue.present(frame);
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
            label: Some("gallery/device"),
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
        present_splash(&device, &queue, &surface);

        let backdrop = Backdrop::from_rgba(
            &device,
            &queue,
            size.width,
            size.height,
            &backdrop_gen::generate_backdrop(size.width, size.height),
        );

        let glass = GlassLayer::new(&device, format, scale);
        let exhibits = build_exhibits([size.width as f32, size.height as f32], scale);

        let ui = EguiFrame::new(&window, format, &device);
        install_cjk_fonts(ui.ctx());

        self.state = Some(WindowState {
            window,
            device,
            queue,
            surface,
            config,
            backdrop,
            glass,
            exhibits,
            scale_base: scale,
            cursor: [size.width as f32 * 0.5, size.height as f32 * 0.5],
            ui,
            show_panel: true,
            fps: 0.0,
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
        for e in &mut state.exhibits {
            e.button.advance(dt);
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

        // ---- UI 阶段：解构字段得到互不相交的借用 ----
        let WindowState {
            window,
            device,
            queue,
            surface: _,
            config,
            backdrop,
            glass,
            exhibits,
            scale_base: _,
            cursor: _,
            ui,
            show_panel,
            fps,
        } = state;

        // 标签快照：run_ui 闭包内 guide 已可变借用 exhibits，标签先拷出
        // 名称与中心（按钮在 UI 阶段不动，快照安全）。
        let labels: Vec<(&'static str, [f32; 2])> = exhibits
            .iter()
            .map(|e| (e.name, e.button.center()))
            .collect();
        let mut guide = Guide { exhibits, fps };
        ui.run_ui(window, |ui| {
            // show_collapsible 会在拖边缘收起时翻转 is_expanded；
            // 先拷出 bool 再写回，避免与闭包对 show_panel 的可变借用冲突。
            let mut show = std::mem::take(show_panel);
            egui::Panel::right("guide")
                .default_size(300.0)
                .show_collapsible(ui, &mut show, |ui| build_ui(ui, &mut guide));
            *show_panel = show;

            // 展品的"身份"层：名称由平台（这里 egui）画在玻璃体之上。
            // 用 painter 直接绘制——不参与 egui 的交互命中，点击仍由
            // 玻璃按钮自己的状态机处理。
            let ppp = window.scale_factor() as f32;
            for (name, center) in labels.iter() {
                ui.ctx()
                    .layer_painter(egui::LayerId::new(
                        egui::Order::Foreground,
                        egui::Id::new(("exhibit-label", *name)),
                    ))
                    .text(
                        egui::pos2(center[0] / ppp, center[1] / ppp),
                        egui::Align2::CENTER_CENTER,
                        *name,
                        egui::FontId::proportional(20.0),
                        egui::Color32::WHITE,
                    );
            }
        });

        // ---- 玻璃阶段 ----
        // 悬停提亮高光——桌面指针的即时反馈。
        let mut controls = [GlassPanel::new([0.0; 2], [0.0; 2], 0.0); EXHIBIT_COUNT];
        for (slot, e) in controls.iter_mut().zip(exhibits.iter_mut()) {
            let hovered = e.button.hovered();
            e.button.style_mut().specular = if hovered {
                e.base_specular + 0.25
            } else {
                e.base_specular
            };
            *slot = e.button.panel();
        }

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        glass.render_static_with_controls(
            queue,
            &mut encoder,
            backdrop,
            &view,
            [config.width as f32, config.height as f32],
            time,
            &controls,
        );

        // ---- egui 叠加 + 合并提交 ----
        let egui_cmds = ui.paint(device, queue, &mut encoder, &view, [config.width, config.height]);
        let mut all = egui_cmds;
        all.push(encoder.finish());
        queue.submit(all);

        queue.present(frame);
        if !self.first_frame_done {
            self.first_frame_done = true;
            log::info!("[startup] first frame: {} ms", self.start.elapsed().as_millis());
        }
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

        state.backdrop = Backdrop::from_rgba(
            &state.device,
            &state.queue,
            width,
            height,
            &backdrop_gen::generate_backdrop(width, height),
        );

        // 展品按视口比例重新排布（弹簧平滑移动过去）。
        let slots = exhibit_slots([width as f32, height as f32]);
        for (e, slot) in state.exhibits.iter_mut().zip(slots) {
            e.button.set_center(slot);
        }
    }

    fn apply_scale(&mut self) {
        let Some(state) = &mut self.state else {
            return;
        };
        // 几何与像素计价材质按比例缩放。
        let scale = state.window.scale_factor() as f32;
        state.glass.apply_scale_factor(scale);
        let ratio = scale / state.scale_base;
        for e in &mut state.exhibits {
            e.button.scale_by(ratio);
        }
        state.scale_base = scale;
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_none() {
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title("Vitreo — gallery（控件展览 · 点击按钮）")
                            .with_inner_size(winit::dpi::LogicalSize::new(1200.0, 800.0)),
                    )
                    .expect("create window"),
            );
            self.init_state(window);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Tab 切换导览面板显隐。必须在喂给 egui 之前处理，
        // 否则 Tab 会被 egui 的焦点导航消费掉。
        if let WindowEvent::KeyboardInput { event: key, .. } = &event {
            use winit::keyboard::{Key, NamedKey};
            if key.state == ElementState::Pressed
                && !key.repeat
                && matches!(key.logical_key, Key::Named(NamedKey::Tab))
            {
                if let Some(state) = &mut self.state {
                    state.show_panel = !state.show_panel;
                }
                return;
            }
        }

        // 剩余事件喂给 egui；被 UI 消费的（拖滑杆、点按钮）不再传给玻璃控件。
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
                    let pos = [position.x as f32, position.y as f32];
                    state.cursor = pos;
                    for e in &mut state.exhibits {
                        e.button.pointer_move(pos);
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button != MouseButton::Left {
                    return;
                }
                if let Some(ws) = &mut self.state {
                    match state {
                        ElementState::Pressed => {
                            // 展品互不重叠；第一个命中的按钮吃掉事件。
                            for e in &mut ws.exhibits {
                                if e.button.pointer_down(ws.cursor) {
                                    break;
                                }
                            }
                        }
                        ElementState::Released => {
                            for e in &mut ws.exhibits {
                                if e.button.pointer_up(ws.cursor) {
                                    e.clicks += 1;
                                }
                            }
                        }
                    }
                }
            }
            WindowEvent::Focused(false) => {
                // 失焦取消按压：否则"按住 → 切走 → 回来抬起"会误计 click。
                if let Some(ws) = &mut self.state {
                    for e in &mut ws.exhibits {
                        e.button.cancel();
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
        first_frame_done: false,
    };

    event_loop.run_app(&mut app).expect("event loop run");
}
