//! # vitreo-egui
//!
//! [`vitreo`](https://docs.rs/vitreo) 的 egui 绑定：egui-wgpu / egui-winit
//! 帧接线与系统 CJK 字体装载。弹簧动画、玻璃面板与合成层都住在
//! [`vitreo`](https://docs.rs/vitreo) 核心里（渲染器无关），本 crate 只保留
//! egui 专属的胶水。
//!
//! ## 核心类型
//!
//! - [`EguiFrame`]——egui 三件套 + 每帧接线（含纹理增量修复）；
//! - [`install_cjk_fonts`]——系统 CJK 字体装载。
//!
//! egui / wgpu / winit 按绑定 crate 所用版本重导出，应用直接
//! `use vitreo_egui::{egui, wgpu}` 即可。
//!
//! ## 帧序配方（同一个 encoder，与示例一致）
//!
//! ```text
//! 输入阶段    hotkey（应用自留）→ EguiFrame::on_window_event → 玻璃拖拽
//! UI 阶段     EguiFrame::run_ui(|ctx| ...)   ← 先跑 UI，它可能改面板参数
//! 动画阶段    GlassLayer::advance(dt)
//! 场景阶段    应用把自己的场景画进 GlassLayer::scene_target_view()（live 模式）
//! 玻璃阶段    GlassLayer::render_static / render_live
//! UI 叠加     EguiFrame::paint → 返回的命令缓冲与玻璃 encoder 一起 submit
//! ```
//!
//! ```no_run
//! # use vitreo_egui::{wgpu, EguiFrame};
//! # use vitreo::GlassLayer;
//! # fn frame(
//! #     device: &wgpu::Device, queue: &wgpu::Queue,
//! #     layer: &mut GlassLayer, ui: &mut EguiFrame,
//! #     surface_view: &wgpu::TextureView, size: [u32; 2], time: f32, dt: f32,
//! # ) {
//! // 1. UI 逻辑（可能改面板参数）
//! ui.run_ui(/* window */ unimplemented!(), |ctx| { /* egui 控件 */ });
//! // 2. 弹簧推进
//! layer.advance(dt);
//! // 3. 玻璃合成（静态背景模式）
//! # let backdrop: vitreo::Backdrop = unimplemented!();
//! let mut encoder = device.create_command_encoder(&Default::default());
//! layer.render_static(queue, &mut encoder, &backdrop, surface_view, [size[0] as f32, size[1] as f32], time);
//! // 4. egui 叠加 + 合并提交
//! let egui_cmds = ui.paint(&device, &queue, &mut encoder, surface_view, size);
//! let mut all = egui_cmds;
//! all.push(encoder.finish());
//! queue.submit(all);
//! # }
//! ```

pub mod fonts;
pub mod frame;

pub use fonts::install_cjk_fonts;
pub use frame::EguiFrame;

// 版本对齐：让应用 `use vitreo_egui::{egui, wgpu, winit}` 即可拿到与
// 绑定 crate 完全一致的版本，避免重复写版本约束。
pub use egui;
pub use wgpu;
pub use winit;
