//! # vitreo
//!
//! 渲染器无关的 Rust 玻璃材质层：一个基于 wgpu 的光学核心（SDF 折射、色散、Fresnel），
//! 任何应用——egui、游戏、编辑器、overlay——都能给任意区域加上 Liquid Glass 级别的玻璃材质。
//!
//! 一句话设计准则：**光学的部分跨平台共享，身份的部分每平台本地化。折射写一次，铬各写各的。**
//!
//! ## 管线
//!
//! `sdRoundedBox` SDF → bevel 高度场 → 法线 → `refract()` 屏幕空间位移 →
//! 每通道独立 eta 色散 → Schlick–Fresnel 边缘高光。IOR 是真实材料常数（默认 ≈1.52，冕牌玻璃）。
//!
//! ## 动画层
//!
//! 弹簧动画同样住在核心里（渲染器无关，纯 CPU 数学）：
//! - [`Spring`] —— 半隐式 Euler 弹簧积分器（smooth / bouncy / snappy 预设）；
//! - [`AnimatedPanel`] —— 中心/尺寸/圆角各挂弹簧，位置弹簧的**解析加速度**
//!   喂给 `GlassPanel::acceleration` 驱动着色器端果冻形变，按压弹簧驱动整体微缩；
//! - [`GlassLayer`] —— 至多 8 块面板 + 合成策略 + 拖拽状态机 + 可选离屏实时背景，
//!   全部以物理像素与秒为单位，与 UI 框架无关。
//!
//! ## 控件
//!
//! [`GlassButton`] —— 胶囊玻璃按钮：悬停/按压/click 判定 + 按压微缩 +
//! 果冻回弹。控件同样是渲染器无关的 CPU 状态机；文字与图标等"身份"
//! 部分由各平台在玻璃体之上自绘。
//!
//! ## 用法
//!
//! ```no_run
//! # use vitreo::{Backdrop, Compositor, GlassPanel, GlassStyle};
//! # fn docs(device: &wgpu::Device, queue: &wgpu::Queue) {
//! // 1. 背景可以是任何 RGBA 纹理：静态图、离屏实时渲染、视频帧。
//! let rgba = vec![0u8; 1600 * 1000 * 4];
//! let backdrop = Backdrop::from_rgba(device, queue, 1600, 1000, &rgba);
//!
//! // 2. 玻璃面板 = 几何 + 材质（静态面板可用 `GlassPanel::new` 便捷构造）。
//! let mut panel = GlassPanel::new([800.0, 500.0], [460.0, 240.0], 64.0);
//! panel.style = GlassStyle::default();
//!
//! // 3. 合成器把背景 + 面板一次性画到目标纹理上。
//! let compositor = Compositor::new(device, wgpu::TextureFormat::Bgra8UnormSrgb);
//! # let _ = (backdrop, panel, compositor);
//! # }
//! ```
//!
//! ## 致谢
//!
//! - [m2-md/liquid-glass-refraction-shader](https://github.com/m2-md/liquid-glass-refraction-shader)（MIT）—
//!   IOR、色散、Fresnel 数学正源；optics / sdf2d 模块与其测试移植自该项目。
//! - [jeantimex/glass-effect-webgpu](https://github.com/jeantimex/glass-effect-webgpu)（MIT）—
//!   WGSL 折射 shader（thickness/bezel/refraction/blur/specular uniform 管线）直接来源。

pub mod animated;
pub mod backdrop;
pub mod compositor;
pub mod control;
pub mod layer;
pub mod optics;
pub mod panel;
pub mod sdf2d;
pub mod spring;
pub mod style;

pub use animated::AnimatedPanel;
pub use backdrop::{Backdrop, LiveBackdrop};
pub use compositor::{Compositor, CompositeStrategy};
pub use control::GlassButton;
pub use layer::GlassLayer;
pub use panel::GlassPanel;
pub use spring::{Spring, SpringConfig};
pub use style::{GlassStyle, JellyStyle, ShadowStyle};
