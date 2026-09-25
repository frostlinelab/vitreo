//! minimal —— Everywhere Glasses 的 P1 验收 demo：
//! 渐变背景 + 可拖玻璃面板，折射 + 色散 + Fresnel 全开。
//!
//! 窗口模式自带 egui 参数面板（Tab 或 H 键切换显示），可实时调节
//! 形状 / 折射 / 色散 / 高光 / 模糊 / 着色 / 阴影，并支持预设、随机与截图。
//!
//! 用法：
//! ```text
//! cargo run -p minimal                       # 窗口模式，拖拽玻璃面板
//! cargo run -p minimal -- --screenshot out.png [--size 1600x1000] [--depth 110]
//! ```

mod backdrop_gen;

use std::sync::Arc;
use std::time::Instant;

use glasses_core::{Backdrop, Compositor, GlassPanel, GlassStyle, ShadowStyle};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

#[derive(Debug)]
struct Args {
    screenshot: Option<String>,
    size: (u32, u32),
    panel_center: Option<(f32, f32)>,
    bevel: f32,
    thickness: f32,
    depth: f32,
    dispersion: f32,
    blur: f32,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            screenshot: None,
            size: (1600, 1000),
            panel_center: None,
            bevel: 40.0,
            thickness: 8.0,
            depth: 110.0,
            dispersion: 0.15,
            blur: 2.5,
        }
    }
}

fn parse_args() -> Args {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .unwrap_or_else(|| panic!("missing value for {arg}"))
        };
        match arg.as_str() {
            "--screenshot" => args.screenshot = Some(value()),
            "--size" => {
                let v = value();
                let (w, h) = v
                    .split_once('x')
                    .unwrap_or_else(|| panic!("--size needs WxH"));
                args.size = (w.parse().unwrap(), h.parse().unwrap());
            }
            "--panel" => {
                let v = value();
                let (x, y) = v
                    .split_once(',')
                    .unwrap_or_else(|| panic!("--panel needs X,Y"));
                args.panel_center = Some((x.parse().unwrap(), y.parse().unwrap()));
            }
            "--bevel" => args.bevel = value().parse().unwrap(),
            "--thickness" => args.thickness = value().parse().unwrap(),
            "--depth" => args.depth = value().parse().unwrap(),
            "--dispersion" => args.dispersion = value().parse().unwrap(),
            "--blur" => args.blur = value().parse().unwrap(),
            other => eprintln!("unknown argument: {other}"),
        }
    }
    args
}

fn demo_style(args: &Args, scale: f32) -> GlassStyle {
    GlassStyle {
        ior: 1.52,
        bevel: args.bevel * scale,
        thickness: args.thickness * scale,
        depth: args.depth * scale,
        dispersion: args.dispersion,
        blur: args.blur * scale,
        specular: 0.85,
        tint: [1.0, 1.0, 1.0],
        tint_opacity: 0.0,
        shadow: ShadowStyle {
            opacity: 0.30,
            blur: 36.0 * scale,
            offset: [0.0, 18.0 * scale],
        },
    }
}

/// 参数预设：名称 -> (尺寸, 圆角, 材质)，尺寸类参数按 DPI 缩放。
fn apply_preset(state: &mut WindowState, name: &str) {
    let scale = state.window.scale_factor() as f32;
    let panel = &mut state.panel;
    let style = &mut panel.style;
    match name {
        "水晶" => {
            panel.size = [440.0 * scale, 260.0 * scale];
            panel.corner_radius = 56.0 * scale;
            *style = GlassStyle {
                ior: 1.62,
                bevel: 30.0 * scale,
                thickness: 12.0 * scale,
                depth: 140.0 * scale,
                dispersion: 0.22,
                blur: 0.0,
                specular: 1.25,
                tint: [0.94, 0.98, 1.0],
                tint_opacity: 0.08,
                shadow: ShadowStyle {
                    opacity: 0.35,
                    blur: 30.0 * scale,
                    offset: [0.0, 14.0 * scale],
                },
            };
        }
        "彩虹棱镜" => {
            panel.size = [420.0 * scale, 240.0 * scale];
            panel.corner_radius = 48.0 * scale;
            *style = GlassStyle {
                ior: 1.75,
                bevel: 46.0 * scale,
                thickness: 10.0 * scale,
                depth: 170.0 * scale,
                dispersion: 0.50,
                blur: 0.0,
                specular: 1.0,
                tint: [1.0, 1.0, 1.0],
                tint_opacity: 0.0,
                shadow: ShadowStyle {
                    opacity: 0.30,
                    blur: 36.0 * scale,
                    offset: [0.0, 16.0 * scale],
                },
            };
        }
        "磨砂" => {
            panel.size = [500.0 * scale, 300.0 * scale];
            panel.corner_radius = 64.0 * scale;
            *style = GlassStyle {
                ior: 1.45,
                bevel: 36.0 * scale,
                thickness: 8.0 * scale,
                depth: 70.0 * scale,
                dispersion: 0.05,
                blur: 18.0 * scale,
                specular: 0.55,
                tint: [1.0, 1.0, 1.0],
                tint_opacity: 0.14,
                shadow: ShadowStyle {
                    opacity: 0.20,
                    blur: 48.0 * scale,
                    offset: [0.0, 20.0 * scale],
                },
            };
        }
        "胶囊" => {
            panel.size = [520.0 * scale, 180.0 * scale];
            panel.corner_radius = 90.0 * scale;
            *style = GlassStyle {
                ior: 1.52,
                bevel: 44.0 * scale,
                thickness: 9.0 * scale,
                depth: 120.0 * scale,
                dispersion: 0.18,
                blur: 1.0 * scale,
                specular: 0.9,
                tint: [1.0, 1.0, 1.0],
                tint_opacity: 0.0,
                shadow: ShadowStyle {
                    opacity: 0.28,
                    blur: 32.0 * scale,
                    offset: [0.0, 18.0 * scale],
                },
            };
        }
        _ => {
            // "默认"
            panel.size = [460.0 * scale, 240.0 * scale];
            panel.corner_radius = 72.0 * scale;
            *style = GlassStyle {
                ior: 1.52,
                bevel: 40.0 * scale,
                thickness: 8.0 * scale,
                depth: 110.0 * scale,
                dispersion: 0.15,
                blur: 2.5 * scale,
                specular: 0.85,
                tint: [1.0, 1.0, 1.0],
                tint_opacity: 0.0,
                shadow: ShadowStyle {
                    opacity: 0.30,
                    blur: 36.0 * scale,
                    offset: [0.0, 18.0 * scale],
                },
            };
        }
    }
}

/// xorshift64：面板"随机"按钮够用的伪随机。
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        Self(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E3779B97F4A7C15),
        )
    }
    fn f32(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }
}

fn randomize_panel(state: &mut WindowState) {
    let scale = state.window.scale_factor() as f32;
    let mut rng = Rng::new();
    let panel = &mut state.panel;
    let style = &mut panel.style;
    panel.size = [
        rng.range(240.0, 640.0) * scale,
        rng.range(140.0, 380.0) * scale,
    ];
    let r_max = panel.size[0].min(panel.size[1]) * 0.5;
    panel.corner_radius = rng.range(0.0, r_max);
    *style = GlassStyle {
        ior: rng.range(1.35, 1.85),
        bevel: rng.range(20.0, 60.0) * scale,
        thickness: rng.range(4.0, 16.0) * scale,
        depth: rng.range(50.0, 200.0) * scale,
        dispersion: rng.range(0.0, 0.5),
        blur: rng.f32().powi(3) * 20.0 * scale,
        specular: rng.range(0.4, 1.4),
        tint: [
            rng.range(0.85, 1.0),
            rng.range(0.85, 1.0),
            rng.range(0.85, 1.0),
        ],
        tint_opacity: rng.range(0.0, 0.2),
        shadow: ShadowStyle {
            opacity: rng.range(0.15, 0.4),
            blur: rng.range(20.0, 50.0) * scale,
            offset: [rng.range(-10.0, 10.0) * scale, rng.range(8.0, 24.0) * scale],
        },
    };
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

/// 把当前玻璃面板渲染为 PNG。CLI 截图与面板"保存截图"按钮共用。
fn render_png(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    compositor: &Compositor,
    backdrop: &Backdrop,
    panel: &GlassPanel,
    size: [u32; 2],
    path: &str,
) -> Result<(), String> {
    let (w, h) = (size[0], size[1]);
    if w == 0 || h == 0 {
        return Err("尺寸为 0".into());
    }

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("screenshot/target"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("screenshot/encoder"),
    });
    compositor.render(
        queue,
        &mut encoder,
        backdrop,
        &view,
        [w as f32, h as f32],
        0.0,
        std::slice::from_ref(panel),
    );

    let stride = (w * 4).div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot/readback"),
        size: stride as u64 * h as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| format!("poll device: {e:?}"))?;
    let data = slice
        .get_mapped_range()
        .map_err(|e| format!("map: {e:?}"))?;

    let file = std::fs::File::create(path).map_err(|e| format!("创建文件 {path}: {e}"))?;
    let mut png_encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    png_encoder.set_color(png::ColorType::Rgba);
    png_encoder.set_depth(png::BitDepth::Eight);
    let mut writer = png_encoder
        .write_header()
        .map_err(|e| format!("png: {e}"))?;
    let mut rows = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let s = (y * stride) as usize;
        rows.extend_from_slice(&data[s..s + (w * 4) as usize]);
    }
    writer
        .write_image_data(&rows)
        .map_err(|e| format!("png: {e}"))?;
    Ok(())
}

/// egui 参数面板内容：形状 / 折射 / 色散高光模糊 / 着色 / 阴影 分组滑杆 + 预设 + 截图。
/// 面板本体（`Panel::right + show_collapsible`）在 render 循环里创建，这里只画内容。
fn build_ui(ui: &mut egui::Ui, state: &mut WindowState) {
    ui.add_space(4.0);
    ui.heading("Everywhere Glasses");
    ui.label(format!(
        "{:.0} fps · 拖动玻璃面板 · Tab 隐藏面板",
        state.fps
    ));
    ui.separator();

    ui.horizontal(|ui| {
        for name in ["默认", "水晶", "彩虹棱镜", "磨砂", "胶囊"] {
            if ui.small_button(name).clicked() {
                apply_preset(state, name);
            }
        }
    });
    ui.horizontal(|ui| {
        if ui
            .button("🎲 随机参数")
            .on_hover_text("在合理范围内随机生成一组参数")
            .clicked()
        {
            randomize_panel(state);
        }
        if ui
            .button("居中玻璃")
            .on_hover_text("把玻璃面板移回视口中心")
            .clicked()
        {
            state.panel.target = [
                state.config.width as f32 * 0.5,
                state.config.height as f32 * 0.5,
            ];
        }
    });
    ui.separator();

    let panel = &mut state.panel;
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
                    .text("背景模糊")
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

    ui.separator();
    if ui.button("📸 保存截图（当前参数）").clicked() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = format!("/tmp/glasses_{stamp}.png");
        let result = render_png(
            &state.device,
            &state.queue,
            &state.offscreen_compositor,
            &state.backdrop,
            &state.panel.panel(),
            [state.config.width, state.config.height],
            &path,
        );
        match result {
            Ok(()) => state.last_screenshot = Some((path, ui.ctx().time() as f32)),
            Err(e) => log::error!("截图失败: {e}"),
        }
    }
    if let Some((path, t)) = &state.last_screenshot {
        if ui.ctx().time() - (*t as f64) < 4.0 {
            ui.label(format!("已保存: {path}"));
        }
    }
}

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
            style: self.style,
        }
    }

    fn lerp_to_target(&mut self, dt: f32) {
        let k = 1.0 - (-dt * 18.0).exp();
        for i in 0..2 {
            self.center[i] += (self.target[i] - self.center[i]) * k;
        }
    }
}

fn main() {
    env_logger::init();
    let args = parse_args();

    if let Some(path) = args.screenshot.clone() {
        run_screenshot(&args, &path);
        return;
    }

    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App {
        args,
        state: None,
        last_frame: Instant::now(),
        start: Instant::now(),
    };

    event_loop.run_app(&mut app).expect("event loop run");
}

struct WindowState {
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    backdrop: Backdrop,
    compositor: Compositor,
    /// 离屏截图专用合成器（固定 Rgba8UnormSrgb，与 surface 格式解耦）。
    offscreen_compositor: Compositor,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    show_panel: bool,
    fps: f32,
    last_screenshot: Option<(String, f32)>,
    panel: PanelState,
    cursor: [f32; 2],
    drag: Option<[f32; 2]>,
}

struct App {
    args: Args,
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
            label: Some("minimal/device"),
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

        let backdrop = Backdrop::from_rgba(
            &device,
            &queue,
            size.width,
            size.height,
            &backdrop_gen::generate_backdrop(size.width, size.height),
        );
        let compositor = Compositor::new(&device, format);

        let center = self
            .args
            .panel_center
            .map(|(x, y)| [x * scale, y * scale])
            .unwrap_or([size.width as f32 * 0.5, size.height as f32 * 0.55]);
        let panel = PanelState {
            center,
            target: center,
            size: [460.0 * scale, 240.0 * scale],
            corner_radius: 72.0 * scale,
            style: demo_style(&self.args, scale),
        };

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
        let offscreen_compositor = Compositor::new(&device, wgpu::TextureFormat::Rgba8UnormSrgb);

        self.state = Some(WindowState {
            window,
            device,
            queue,
            surface,
            config,
            backdrop,
            compositor,
            offscreen_compositor,
            egui_ctx,
            egui_state,
            egui_renderer,
            show_panel: true,
            fps: 0.0,
            last_screenshot: None,
            panel,
            cursor: center,
            drag: None,
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
        state.panel.lerp_to_target(dt);

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

        // ---- egui：先跑 UI 逻辑（可能修改面板参数），再统一编码 ----
        // Context 内部是 Arc，clone 廉价，且让 build_ui 能独占整个 state。
        let egui_ctx = state.egui_ctx.clone();
        let input = state.egui_state.take_egui_input(&state.window);
        let full_output = egui_ctx.run_ui(input, |ui| {
            // show_collapsible 会在拖边缘收起时翻转 is_expanded；
            // 先拷出 bool 再写回，避免与闭包对 state 的可变借用冲突。
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

        let pixels_per_point = state.window.scale_factor() as f32;
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [state.config.width, state.config.height],
            pixels_per_point,
        };

        let panels = [state.panel.panel()];
        let mut encoder = state
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        state.compositor.render(
            &state.queue,
            &mut encoder,
            &state.backdrop,
            &view,
            [state.config.width as f32, state.config.height as f32],
            time,
            &panels,
        );

        if !clipped_primitives.is_empty() {
            let egui_cmds = state.egui_renderer.update_buffers(
                &state.device,
                &state.queue,
                &mut encoder,
                &clipped_primitives,
                &screen_descriptor,
            );
            for (id, deltas) in &full_output.textures_delta.set {
                for delta in deltas {
                    state
                        .egui_renderer
                        .update_texture(&state.device, &state.queue, *id, delta);
                }
            }
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
            for id in &full_output.textures_delta.free {
                state.egui_renderer.free_texture(id);
            }

            let mut all = egui_cmds;
            all.push(encoder.finish());
            state.queue.submit(all);
        } else {
            state.queue.submit(Some(encoder.finish()));
        }
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

        state.backdrop = Backdrop::from_rgba(
            &state.device,
            &state.queue,
            width,
            height,
            &backdrop_gen::generate_backdrop(width, height),
        );

        // 面板留在视口内
        for i in 0..2 {
            let lim = [width as f32, height as f32][i];
            state.panel.target[i] = state.panel.target[i].clamp(0.0, lim);
            state.panel.center[i] = state.panel.center[i].clamp(0.0, lim);
        }
    }

    fn apply_scale(&mut self) {
        let Some(state) = &mut self.state else {
            return;
        };
        let scale = state.window.scale_factor() as f32;
        // 只按新 DPI 缩放几何尺寸；材质参数保留用户在面板里的调整。
        state.panel.size = [460.0 * scale, 240.0 * scale];
        state.panel.corner_radius = 72.0 * scale;
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_none() {
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title("Everywhere Glasses — minimal（拖玻璃 · Tab 调参数）")
                            .with_inner_size(winit::dpi::LogicalSize::new(1000.0, 640.0)),
                    )
                    .expect("create window"),
            );
            self.init_state(window);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Tab / H 切换参数面板显隐。必须在喂给 egui 之前处理，
        // 否则 Tab 会被 egui 的焦点导航消费掉，H 可能落入聚焦控件。
        if let WindowEvent::KeyboardInput { event: key, .. } = &event {
            use winit::keyboard::{Key, NamedKey};
            let toggle = matches!(key.logical_key, Key::Named(NamedKey::Tab))
                || matches!(&key.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("h"));
            if key.state == ElementState::Pressed && !key.repeat && toggle {
                if let Some(state) = &mut self.state {
                    state.show_panel = !state.show_panel;
                }
                return;
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
                    if let Some(grab) = state.drag {
                        state.panel.target = [state.cursor[0] - grab[0], state.cursor[1] - grab[1]];
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
                            if ws.panel.panel().contains(ws.cursor) {
                                ws.drag = Some([
                                    ws.cursor[0] - ws.panel.center[0],
                                    ws.cursor[1] - ws.panel.center[1],
                                ]);
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

fn run_screenshot(args: &Args, path: &str) {
    pollster::block_on(async {
        let (w, h) = args.size;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .expect("no suitable GPU adapter");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("screenshot/device"),
                ..Default::default()
            })
            .await
            .expect("request device");

        let backdrop = Backdrop::from_rgba(
            &device,
            &queue,
            w,
            h,
            &backdrop_gen::generate_backdrop(w, h),
        );
        let compositor = Compositor::new(&device, wgpu::TextureFormat::Rgba8UnormSrgb);
        let panel = GlassPanel {
            center: args
                .panel_center
                .map(|(x, y)| [x, y])
                .unwrap_or([w as f32 * 0.5, h as f32 * 0.55]),
            size: [460.0, 240.0],
            corner_radius: 72.0,
            style: demo_style(args, 1.0),
        };

        render_png(
            &device,
            &queue,
            &compositor,
            &backdrop,
            &panel,
            [w, h],
            path,
        )
        .expect("render png");
    });

    println!("screenshot saved: {path}");
}
