//! # vitreo-egui
//!
//! [`vitreo`](https://docs.rs/vitreo) 的 egui 绑定：弹簧驱动的玻璃面板
//! （带果冻形变）+ egui-wgpu/winit 帧接线的成套封装。
//!
//! ## 三件核心类型
//!
//! - [`Spring`] / [`SpringConfig`]——弹簧积分器（平滑跟随与回弹手感）；
//! - [`AnimatedPanel`]——弹簧化玻璃面板，速度直接喂给 `GlassPanel::velocity`
//!   驱动着色器端的果冻形变；
//! - [`GlassLayer`]——多面板合成层（Stack/Merge、拖拽、DPI、LiveBackdrop）。
//!
//! `EguiFrame`（egui-wgpu/winit 帧接线）在后续提交落地。
//!
//! ## 帧序配方（与示例一致，同一个 encoder）
//!
//! 1. 先跑 UI 逻辑（它可能改面板参数）；
//! 2. [`GlassLayer::advance`] 推进弹簧；
//! 3. 应用渲染自己的场景（可选，进 [`GlassLayer::scene_target_view`]）；
//! 4. [`GlassLayer::render_static`] / [`GlassLayer::render_live`]——玻璃合成进 surface；
//! 5. egui 以 `LoadOp::Load` 叠加在同一目标上；
//! 6. 应用 submit。

pub mod animated;
pub mod layer;
pub mod spring;

pub use animated::AnimatedPanel;
pub use layer::GlassLayer;
pub use spring::{Spring, SpringConfig};
