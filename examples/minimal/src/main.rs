//! minimal —— Vitreo 的 P1 验收 demo：
//! 渐变背景 + 可拖玻璃面板，折射 + 色散 + Fresnel 全开。
//!
//! P3 起弹簧驱动面板（拖拽带果冻回弹 + 按压微缩）住在 vitreo 核心，
//! egui/winit/wgpu 接线来自 `vitreo-egui` 绑定。Tab 或 H 键切换参数面板。
//!
//! 用法：
//! ```text
//! cargo run -p minimal                       # 窗口模式，拖拽玻璃面板
//! cargo run -p minimal -- --image photo.jpg  # 以图片为背景
//! cargo run -p minimal -- --screenshot out.png [--size 1600x1000] [--depth 110]
//! ```

mod backdrop_gen;
mod backdrop_image;

use std::sync::Arc;
use std::time::Instant;

use vitreo::{
    AnimatedPanel, Backdrop, Compositor, GlassLayer, GlassPanel, GlassStyle, JellyStyle,
    ShadowStyle, SpringConfig,
};
use vitreo_egui::{egui, install_cjk_fonts, EguiFrame};
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
    image: Option<String>,
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
            image: None,
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
            "--image" => args.image = Some(value()),
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
        jelly: JellyStyle::default(),
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

/// build_ui 需要的可变借用集合 —— 从 WindowState 逐字段解构而来，
/// 避免闭包捕获整个 state 与 `EguiFrame::run_ui` 的借用冲突。
/// 换背景操作涉及 `&mut Backdrop`（整块替换），从 UI 推迟到 run_ui
/// 结束后处理，这里只收集意图。
struct Demo<'a> {
    glass: &'a mut GlassLayer,
    window: &'a Arc<Window>,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    offscreen_compositor: &'a mut Option<Compositor>,
    backdrop: &'a Backdrop,
    size: [u32; 2],
    fps: &'a mut f32,
    backdrop_image: &'a Option<(std::path::PathBuf, image::RgbaImage)>,
    backdrop_error: &'a mut Option<(String, f32)>,
    last_screenshot: &'a mut Option<(String, f32)>,
    pending_load: Option<std::path::PathBuf>,
    clear_backdrop: bool,
}

impl Demo<'_> {
    fn panel(&mut self) -> &mut AnimatedPanel {
        self.glass.panel_mut(0).expect("minimal 只有一块面板")
    }

    fn scale(&self) -> f32 {
        self.window.scale_factor() as f32
    }
}

/// 参数预设：名称 -> (尺寸, 圆角, 材质)，尺寸类参数按 DPI 缩放。
/// 尺寸与圆角走弹簧目标，材质直接替换。
fn apply_preset(d: &mut Demo, name: &str) {
    let scale = d.scale();
    let panel = d.panel();
    match name {
        "水晶" => {
            panel.set_target_size([440.0 * scale, 260.0 * scale]);
            panel.set_target_corner_radius(56.0 * scale);
            *panel.style_mut() = GlassStyle {
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
                jelly: JellyStyle::default(),
            };
        }
        "彩虹棱镜" => {
            panel.set_target_size([420.0 * scale, 240.0 * scale]);
            panel.set_target_corner_radius(48.0 * scale);
            *panel.style_mut() = GlassStyle {
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
                jelly: JellyStyle::default(),
            };
        }
        "磨砂" => {
            panel.set_target_size([500.0 * scale, 300.0 * scale]);
            panel.set_target_corner_radius(64.0 * scale);
            *panel.style_mut() = GlassStyle {
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
                jelly: JellyStyle::default(),
            };
        }
        "胶囊" => {
            panel.set_target_size([520.0 * scale, 180.0 * scale]);
            panel.set_target_corner_radius(90.0 * scale);
            *panel.style_mut() = GlassStyle {
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
                jelly: JellyStyle::default(),
            };
        }
        _ => {
            // "默认"
            panel.set_target_size([460.0 * scale, 240.0 * scale]);
            panel.set_target_corner_radius(72.0 * scale);
            *panel.style_mut() = GlassStyle {
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
                jelly: JellyStyle::default(),
            };
        }
    }
}

fn randomize_panel(d: &mut Demo) {
    let scale = d.scale();
    let mut rng = Rng::new();
    let panel = d.panel();
    panel.set_target_size([
        rng.range(240.0, 640.0) * scale,
        rng.range(140.0, 380.0) * scale,
    ]);
    let r_max = panel.target_size()[0].min(panel.target_size()[1]) * 0.5;
    panel.set_target_corner_radius(rng.range(0.0, r_max));
    *panel.style_mut() = GlassStyle {
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
        jelly: JellyStyle {
            stretch_gain: rng.range(2e-6, 12e-6),
            stretch_max: rng.range(0.08, 0.2),
            press_squash: rng.range(0.03, 0.1),
            normal_lag: rng.range(0.4, 0.8),
        },
    };
}

/// 按当前窗口尺寸重建背景纹理：有自定义图片用图片（cover 裁剪），否则用程序化渐变。
fn rebuild_backdrop(state: &mut WindowState) {
    rebuild_backdrop_into(
        &mut state.backdrop,
        &state.device,
        &state.queue,
        &state.backdrop_image,
        state.config.width,
        state.config.height,
    );
}

/// [`rebuild_backdrop`] 的字段级版本：render 循环里字段已解构，
/// 拿不到整个 WindowState。
fn rebuild_backdrop_into(
    backdrop: &mut Backdrop,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &Option<(std::path::PathBuf, image::RgbaImage)>,
    w: u32,
    h: u32,
) {
    let rgba = match image {
        Some((_, img)) => backdrop_image::fit_cover(img, w, h),
        None => backdrop_gen::generate_backdrop(w, h),
    };
    *backdrop = Backdrop::from_rgba(device, queue, w, h, &rgba);
}

/// 加载图片作为背景；失败时在面板上显示 4 秒错误提示。
fn load_backdrop_file(state: &mut WindowState, path: &std::path::Path) {
    match backdrop_image::load_image_rgba(path) {
        Ok(img) => {
            log::info!("背景图片已加载: {}", path.display());
            state.backdrop_image = Some((path.to_path_buf(), img));
            rebuild_backdrop(state);
        }
        Err(e) => {
            log::error!("{e}");
            state.backdrop_error = Some((e, state.ui.ctx().time() as f32));
        }
    }
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
        vitreo::CompositeStrategy::Stack,
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
/// 形状滑杆写的是弹簧**目标值**——拖动时玻璃平滑地长过去，而不是跳变。
fn build_ui(ui: &mut egui::Ui, d: &mut Demo) {
    ui.add_space(4.0);
    ui.heading("Vitreo");
    ui.label(format!(
        "{:.0} fps · 拖动玻璃面板 · Tab 隐藏面板",
        *d.fps
    ));
    ui.separator();

    ui.horizontal(|ui| {
        for name in ["默认", "水晶", "彩虹棱镜", "磨砂", "胶囊"] {
            if ui.small_button(name).clicked() {
                apply_preset(d, name);
            }
        }
    });
    ui.horizontal(|ui| {
        if ui
            .button("🎲 随机参数")
            .on_hover_text("在合理范围内随机生成一组参数")
            .clicked()
        {
            randomize_panel(d);
        }
        if ui
            .button("居中玻璃")
            .on_hover_text("把玻璃面板移回视口中心")
            .clicked()
        {
            let target = [d.size[0] as f32 * 0.5, d.size[1] as f32 * 0.5];
            d.panel().set_target_center(target);
        }
    });
    ui.horizontal(|ui| {
        if ui
            .button("🖼 背景图片…")
            .on_hover_text("选择 PNG / JPG 作为背景；也可以直接把图片文件拖进窗口")
            .clicked()
        {
            let pick = rfd::FileDialog::new()
                .set_title("选择背景图片")
                .add_filter("图片", &["png", "jpg", "jpeg"])
                .set_parent(d.window.as_ref())
                .pick_file();
            if let Some(path) = pick {
                d.pending_load = Some(path);
            }
        }
        if ui
            .button("默认背景")
            .on_hover_text("恢复程序化生成的渐变网格背景")
            .clicked()
        {
            d.clear_backdrop = true;
        }
    });
    if let Some((path, _)) = d.backdrop_image {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("(未知文件)");
        ui.label(egui::RichText::new(format!("背景: {name}")).small().weak());
    }
    if let Some((err, t)) = &*d.backdrop_error {
        if ui.ctx().time() - (*t as f64) < 4.0 {
            ui.label(
                egui::RichText::new(format!("⚠ {err}"))
                    .small()
                    .color(egui::Color32::LIGHT_RED),
            );
        }
    }
    ui.separator();

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

    egui::CollapsingHeader::new("果冻 · 弹簧")
        .default_open(true)
        .show(ui, |ui| {
            {
                let style = d.panel().style_mut();
                // 增益真实量级 ~6e-6 s²/px，滑杆按 ×10⁻⁶ 计价便于读数。
                let mut gain = style.jelly.stretch_gain * 1e6;
                ui.add(
                    egui::Slider::new(&mut gain, 0.0..=20.0)
                        .text("拉伸增益 (×10⁻⁶ s²/px)")
                        .clamping(egui::SliderClamping::Edits),
                );
                style.jelly.stretch_gain = gain * 1e-6;
                ui.add(
                    egui::Slider::new(&mut style.jelly.stretch_max, 0.0..=0.3)
                        .text("拉伸上限")
                        .clamping(egui::SliderClamping::Edits),
                );
                ui.add(
                    egui::Slider::new(&mut style.jelly.press_squash, 0.0..=0.15)
                        .text("按压微缩")
                        .clamping(egui::SliderClamping::Edits),
                );
                ui.add(
                    egui::Slider::new(&mut style.jelly.normal_lag, 0.0..=1.0)
                        .text("法线滞后")
                        .clamping(egui::SliderClamping::Edits),
                );
            }
            let mut cfg = d.panel().spring_config();
            ui.add(
                egui::Slider::new(&mut cfg.stiffness, 40.0..=600.0)
                    .text("弹簧刚度")
                    .clamping(egui::SliderClamping::Edits),
            );
            ui.add(
                egui::Slider::new(&mut cfg.damping, 4.0..=60.0)
                    .text("弹簧阻尼")
                    .clamping(egui::SliderClamping::Edits),
            );
            ui.label(
                egui::RichText::new(format!(
                    "阻尼比 ζ = {:.2}（<1 回弹 · =1 无过冲）",
                    cfg.damping_ratio()
                ))
                .small()
                .weak(),
            );
            d.panel().set_spring_config(cfg);
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
                    .text("背景模糊")
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

    ui.separator();
    if ui.button("📸 保存截图（当前参数）").clicked() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = format!("/tmp/glasses_{stamp}.png");
        // 首次点击才创建离屏合成器（含管线编译），启动不为截图付费。
        let compositor = d
            .offscreen_compositor
            .get_or_insert_with(|| Compositor::new(d.device, wgpu::TextureFormat::Rgba8UnormSrgb));
        let result = render_png(
            d.device,
            d.queue,
            compositor,
            d.backdrop,
            &d.glass.panels()[0].panel(),
            d.size,
            &path,
        );
        match result {
            Ok(()) => *d.last_screenshot = Some((path, ui.ctx().time() as f32)),
            Err(e) => log::error!("截图失败: {e}"),
        }
    }
    if let Some((path, t)) = &*d.last_screenshot {
        if ui.ctx().time() - (*t as f64) < 4.0 {
            ui.label(format!("已保存: {path}"));
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
    /// 自定义背景（原始解码图，窗口 resize 时按 cover 重新裁剪）。
    backdrop_image: Option<(std::path::PathBuf, image::RgbaImage)>,
    /// 最近一次背景加载错误（时间戳用于 4 秒后自动消失）。
    backdrop_error: Option<(String, f32)>,
    /// 离屏截图专用合成器（固定 Rgba8UnormSrgb，与 surface 格式解耦）。
    /// 惰性创建：点"保存截图"时才建管线，不占启动时间。
    offscreen_compositor: Option<Compositor>,
    /// 弹簧玻璃层：唯一的面板 + 合成器 + 拖拽状态机。
    glass: GlassLayer,
    /// egui 三件套 + 帧接线。
    ui: EguiFrame,
    show_panel: bool,
    fps: f32,
    last_screenshot: Option<(String, f32)>,
}

struct App {
    args: Args,
    state: Option<WindowState>,
    last_frame: Instant,
    start: Instant,
    first_frame_done: bool,
}

/// 启动闪屏帧：surface 配置好就立刻呈现背景主题色。
/// 之后的背景生成、（冷 Metal 缓存下的）管线编译、字体装载都发生在
/// 这帧之后——窗口从出现的第一刻起就是主题色，而不是系统白窗。
fn present_splash(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    surface: &wgpu::Surface<'_>,
) {
    let frame = match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame)
        | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
        _ => return,
    };
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("minimal/splash"),
    });
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("minimal/splash-pass"),
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
        present_splash(&device, &queue, &surface);

        let backdrop = Backdrop::from_rgba(
            &device,
            &queue,
            size.width,
            size.height,
            &backdrop_gen::generate_backdrop(size.width, size.height),
        );

        let center = self
            .args
            .panel_center
            .map(|(x, y)| [x * scale, y * scale])
            .unwrap_or([size.width as f32 * 0.5, size.height as f32 * 0.55]);
        let mut panel = GlassPanel::new(center, [460.0 * scale, 240.0 * scale], 72.0 * scale);
        panel.style = demo_style(&self.args, scale);

        // bouncy 预设：拖拽放手后带一次果冻回弹，顺带展示 shader 级形变。
        let mut glass = GlassLayer::new(&device, format, scale);
        glass
            .add_panel(panel, SpringConfig::bouncy())
            .expect("single panel fits");

        let ui = EguiFrame::new(&window, format, &device);
        install_cjk_fonts(ui.ctx());

        self.state = Some(WindowState {
            window,
            device,
            queue,
            surface,
            config,
            backdrop,
            backdrop_image: None,
            backdrop_error: None,
            offscreen_compositor: None,
            glass,
            ui,
            show_panel: true,
            fps: 0.0,
            last_screenshot: None,
        });

        // --image：启动时直接换背景（失败则回退默认背景并提示）
        if let Some(path) = self.args.image.clone() {
            if let Some(state) = &mut self.state {
                load_backdrop_file(state, std::path::Path::new(&path));
            }
        }
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

        // ---- UI 阶段：解构字段得到互不相交的借用（run_ui 的闭包要改玻璃参数）----
        let WindowState {
            window,
            device,
            queue,
            surface: _,
            config,
            backdrop,
            backdrop_image,
            backdrop_error,
            offscreen_compositor,
            glass,
            ui,
            show_panel,
            fps,
            last_screenshot,
        } = state;

        let mut demo = Demo {
            glass,
            window,
            device,
            queue,
            offscreen_compositor,
            backdrop,
            size: [config.width, config.height],
            fps,
            backdrop_image,
            backdrop_error,
            last_screenshot,
            pending_load: None,
            clear_backdrop: false,
        };
        ui.run_ui(window, |ui| {
            // show_collapsible 会在拖边缘收起时翻转 is_expanded；
            // 先拷出 bool 再写回，避免与闭包对 show_panel 的可变借用冲突。
            let mut show = std::mem::take(show_panel);
            egui::Panel::right("controls")
                .default_size(300.0)
                .show_collapsible(ui, &mut show, |ui| build_ui(ui, &mut demo));
            *show_panel = show;
        });

        // UI 期间请求的换背景：整块替换 &mut Backdrop，等 demo 的字段借用
        // 结束后再做（时间戳用本帧 egui 的时钟）。
        let Demo {
            pending_load,
            clear_backdrop,
            ..
        } = demo;
        if clear_backdrop {
            *backdrop_image = None;
            rebuild_backdrop_into(backdrop, device, queue, backdrop_image, config.width, config.height);
        } else if let Some(path) = pending_load {
            match backdrop_image::load_image_rgba(&path) {
                Ok(img) => {
                    log::info!("背景图片已加载: {}", path.display());
                    *backdrop_image = Some((path, img));
                    rebuild_backdrop_into(backdrop, device, queue, backdrop_image, config.width, config.height);
                }
                Err(e) => {
                    log::error!("{e}");
                    *backdrop_error = Some((e, ui.ctx().time() as f32));
                }
            }
        }

        // ---- 玻璃阶段 ----
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        glass.render_static(
            queue,
            &mut encoder,
            backdrop,
            &view,
            [config.width as f32, config.height as f32],
            time,
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

        rebuild_backdrop(state);

        // 面板留在视口内
        state.glass.clamp_panels_to([width as f32, height as f32]);
    }

    fn apply_scale(&mut self) {
        let Some(state) = &mut self.state else {
            return;
        };
        // 几何与像素计价材质按比例缩放；用户调过的物理量保持不动。
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
                            .with_title("Vitreo — minimal（拖玻璃 · Tab 调参数）")
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
            if state.ui.on_window_event(&state.window, &event) {
                return;
            }
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::Resized(size) => self.resize(size.width, size.height),
            WindowEvent::ScaleFactorChanged { .. } => self.apply_scale(),
            WindowEvent::DroppedFile(path) => {
                if let Some(ws) = &mut self.state {
                    load_backdrop_file(ws, &path);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(state) = &mut self.state {
                    let pos = [position.x as f32, position.y as f32];
                    state.glass.pointer_moved(pos);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button != MouseButton::Left {
                    return;
                }
                if let Some(ws) = &mut self.state {
                    match state {
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

        let rgba = match &args.image {
            Some(p) => match backdrop_image::load_image_rgba(std::path::Path::new(p)) {
                Ok(img) => backdrop_image::fit_cover(&img, w, h),
                Err(e) => {
                    eprintln!("{e}，改用默认背景");
                    backdrop_gen::generate_backdrop(w, h)
                }
            },
            None => backdrop_gen::generate_backdrop(w, h),
        };
        let backdrop = Backdrop::from_rgba(&device, &queue, w, h, &rgba);
        let compositor = Compositor::new(&device, wgpu::TextureFormat::Rgba8UnormSrgb);
        let mut panel = GlassPanel::new(
            args.panel_center
                .map(|(x, y)| [x, y])
                .unwrap_or([w as f32 * 0.5, h as f32 * 0.55]),
            [460.0, 240.0],
            72.0,
        );
        panel.style = demo_style(args, 1.0);

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
        first_frame_done: false,
    };

    event_loop.run_app(&mut app).expect("event loop run");
}
