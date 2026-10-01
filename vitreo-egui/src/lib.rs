//! # vitreo-egui
//!
//! [`vitreo`](https://docs.rs/vitreo) 的 egui 绑定：弹簧驱动的玻璃面板
//! （带果冻形变）+ egui-wgpu/winit 帧接线的成套封装。
//!
//! ## 帧序配方（与示例一致，同一个 encoder）
//!
//! 1. [`EguiFrame::run_ui`]——先跑 UI 逻辑（它可能改面板参数）；
//! 2. 应用渲染自己的场景（可选，进 [`LiveBackdrop`] 的离屏目标）；
//! 3. [`GlassLayer::render_static`] / `render_live`——玻璃合成进 surface；
//! 4. [`EguiFrame::paint`]——egui 以 `LoadOp::Load` 叠加在同一目标上；
//! 5. 应用把返回的命令缓冲与玻璃 encoder 一起 submit。
//!
//! （尚未完成：`EguiFrame` / `GlassLayer` 在后续提交中落地。）

pub mod spring;

pub use spring::{Spring, SpringConfig};
