//! 多面板合成器：把背景纹理 + 玻璃面板一次性画到目标纹理上。

use std::num::NonZeroU64;

use crate::backdrop::Backdrop;
use crate::panel::{GlassPanel, PanelUniform};

const GLASS_WGSL: &str = include_str!("glass.wgsl");

/// 合成器支持的最大面板数。
pub const MAX_PANELS: usize = 8;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GlobalsUniform {
    viewport: [f32; 2],
    time: f32,
    panel_count: u32,
    strategy: u32,
    _pad: [f32; 3], // WGSL uniform 地址空间要求 struct 大小为 16 的倍数
}

/// 多面板重叠区域的合成策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompositeStrategy {
    /// 逐层覆盖（默认）：按数组顺序混色，下标越靠后越靠上层。
    /// 重叠区显示最上层面板对**背景**的折射，下层玻璃在重叠处不可见。
    #[default]
    Stack,
    /// 并集融合：重叠面板按并集 SDF 融合成一块连续玻璃，
    /// 倒角高光环只出现在融合后的外轮廓，内部接缝消失
    /// （法线来自并集高度场，见 `docs/optics.*.md` 的多面板一节）。
    /// 材质参数取覆盖该像素的最上层面板。
    Merge,
}

impl CompositeStrategy {
    fn to_u32(self) -> u32 {
        match self {
            CompositeStrategy::Stack => 0,
            CompositeStrategy::Merge => 1,
        }
    }
}

/// 玻璃合成器。
///
/// 目标纹理格式应当是 sRGB 编码（如 `Bgra8UnormSrgb`），
/// shader 内部全程在线性空间工作，由目标格式负责编码。
/// 重叠面板的合成方式由 [`CompositeStrategy`] 决定。
pub struct Compositor {
    device: wgpu::Device,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    globals_buffer: wgpu::Buffer,
    panels_buffer: wgpu::Buffer,
}

impl Compositor {
    /// `target_format` 是最终合成目标（通常是 surface）的纹理格式。
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glasses/glass.wgsl"),
            source: wgpu::ShaderSource::Wgsl(GLASS_WGSL.into()),
        });

        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glasses/globals"),
            size: std::mem::size_of::<GlobalsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let panels_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glasses/panels"),
            size: PanelUniform::SIZE as u64 * MAX_PANELS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glasses/bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<GlobalsUniform>() as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            PanelUniform::SIZE as u64 * MAX_PANELS as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("glasses/backdrop-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glasses/pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glasses/composite"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
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

        Self {
            device: device.clone(),
            pipeline,
            bind_group_layout,
            sampler,
            globals_buffer,
            panels_buffer,
        }
    }

    /// 把 `backdrop` + `panels` 合成到 `target`。
    ///
    /// - `viewport`：目标尺寸（物理像素），与 `target` 一致；
    /// - `strategy`：重叠面板的合成策略（[`CompositeStrategy::Stack`] / [`CompositeStrategy::Merge`]）；
    /// - `panels`：超出 [`MAX_PANELS`] 的部分被忽略。
    // GPU 提交上下文 + 帧状态，8 个参数各自独立，打包反而增加调用方样板。
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        backdrop: &Backdrop,
        target: &wgpu::TextureView,
        viewport: [f32; 2],
        time: f32,
        strategy: CompositeStrategy,
        panels: &[GlassPanel],
    ) {
        let count = panels.len().min(MAX_PANELS);

        let globals = GlobalsUniform {
            viewport,
            time,
            panel_count: count as u32,
            strategy: strategy.to_u32(),
            _pad: [0.0; 3],
        };
        queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));

        if count > 0 {
            let uniforms: Vec<PanelUniform> =
                panels[..count].iter().map(|p| p.to_uniform()).collect();
            queue.write_buffer(&self.panels_buffer, 0, bytemuck::cast_slice(&uniforms));
        }

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glasses/bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.globals_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.panels_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(backdrop.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("glasses/composite-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..4, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 与 glass.wgsl 的 Globals 逐字节一致：
    // viewport @0 vec2f, time @8, panel_count @12, strategy @16, _pad @20..32。
    #[test]
    fn globals_layout_matches_wgsl() {
        assert_eq!(std::mem::offset_of!(GlobalsUniform, viewport), 0);
        assert_eq!(std::mem::offset_of!(GlobalsUniform, time), 8);
        assert_eq!(std::mem::offset_of!(GlobalsUniform, panel_count), 12);
        assert_eq!(std::mem::offset_of!(GlobalsUniform, strategy), 16);
        assert_eq!(std::mem::size_of::<GlobalsUniform>(), 32);
    }

    // 判别值被 shader 用来分支，不能随手改。
    #[test]
    fn strategy_discriminants_match_wgsl_constants() {
        assert_eq!(CompositeStrategy::Stack.to_u32(), 0);
        assert_eq!(CompositeStrategy::Merge.to_u32(), 1);
    }
}
