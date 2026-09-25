//! minimal —— Everywhere Glasses 的 P1 验收 demo：
//! 渐变背景 + 可拖玻璃面板，折射 + 色散 + Fresnel 全开。
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
        let mut value = || it.next().unwrap_or_else(|| panic!("missing value for {arg}"));
        match arg.as_str() {
            "--screenshot" => args.screenshot = Some(value()),
            "--size" => {
                let v = value();
                let (w, h) = v.split_once('x').unwrap_or_else(|| panic!("--size needs WxH"));
                args.size = (w.parse().unwrap(), h.parse().unwrap());
            }
            "--panel" => {
                let v = value();
                let (x, y) = v.split_once(',').unwrap_or_else(|| panic!("--panel needs X,Y"));
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

        self.state = Some(WindowState {
            window,
            device,
            queue,
            surface,
            config,
            backdrop,
            compositor,
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

        state.queue.submit(Some(encoder.finish()));
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
        state.panel.size = [460.0 * scale, 240.0 * scale];
        state.panel.corner_radius = 72.0 * scale;
        state.panel.style = demo_style(&self.args, scale);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_none() {
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title("Everywhere Glasses — minimal（拖动玻璃面板）")
                            .with_inner_size(winit::dpi::LogicalSize::new(1000.0, 640.0)),
                    )
                    .expect("create window"),
            );
            self.init_state(window);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::Resized(size) => self.resize(size.width, size.height),
            WindowEvent::ScaleFactorChanged { .. } => self.apply_scale(),
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(state) = &mut self.state {
                    state.cursor = [position.x as f32, position.y as f32];
                    if let Some(grab) = state.drag {
                        state.panel.target =
                            [state.cursor[0] - grab[0], state.cursor[1] - grab[1]];
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

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("screenshot/encoder"),
        });
        compositor.render(
            &queue,
            &mut encoder,
            &backdrop,
            &view,
            [w as f32, h as f32],
            0.0,
            &[panel],
        );

        let stride = (w * 4 + 255) / 256 * 256;
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
            .expect("poll device");
        let data = slice.get_mapped_range().expect("mapped range");

        let file = std::fs::File::create(path).expect("create output file");
        let mut png_encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        png_encoder.set_color(png::ColorType::Rgba);
        png_encoder.set_depth(png::BitDepth::Eight);
        let mut writer = png_encoder.write_header().expect("png header");
        let mut rows = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let s = (y * stride) as usize;
            rows.extend_from_slice(&data[s..s + (w * 4) as usize]);
        }
        writer.write_image_data(&rows).expect("png data");
    });

    println!("screenshot saved: {path}");
}
