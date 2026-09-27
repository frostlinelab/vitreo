//! 背景：静态纹理 / 离屏实时纹理 / 视频帧。
//!
//! 对合成器而言，背景只是一个带 mipmap 的 sRGB 纹理。
//! [`Backdrop::from_rgba`] 负责上传并生成完整的 mip 链（CPU 端线性空间盒滤波），
//! 供 shader 做基于 lod 的背景模糊。
//!
//! 每帧都在变化的"实时背景"请用 [`LiveBackdrop`]：应用把自己的场景
//! 渲染进它的 mip-0 渲染目标，再在 GPU 上逐级生成 mip 链，交给合成器采样。

/// 一块背景纹理（含 mip 链）。
pub struct Backdrop {
    view: wgpu::TextureView,
    size: [u32; 2],
}

impl Backdrop {
    /// 从 RGBA8（sRGB 编码）数据创建背景，自动生成 mip 链。
    ///
    /// `rgba` 长度必须为 `width * height * 4`。
    pub fn from_rgba(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Self {
        assert_eq!(
            rgba.len(),
            (width * height * 4) as usize,
            "backdrop rgba length mismatch"
        );
        assert!(width > 0 && height > 0);

        let mip_level_count = 1 + width.max(height).ilog2();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glasses-backdrop"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // 逐级生成 mip：2×2 盒滤波，在线性空间进行（sRGB 空间直接滤会发暗）。
        let mut level_data: Vec<Vec<u8>> = Vec::with_capacity(mip_level_count as usize);
        level_data.push(rgba.to_vec());
        let mut w = width;
        let mut h = height;
        for _ in 1..mip_level_count {
            let (next, nw, nh) = downsample(level_data.last().unwrap(), w, h);
            level_data.push(next);
            w = nw;
            h = nh;
        }

        for (level, data) in level_data.iter().enumerate() {
            let lw = (width >> level).max(1);
            let lh = (height >> level).max(1);
            let bytes_per_row = align_256(lw * 4);
            let mut padded = Vec::with_capacity((bytes_per_row * lh) as usize);
            for y in 0..lh {
                let start = (y * lw * 4) as usize;
                padded.extend_from_slice(&data[start..start + (lw * 4) as usize]);
                padded.resize(((y + 1) * bytes_per_row) as usize, 0);
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &padded,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(lh),
                },
                wgpu::Extent3d {
                    width: lw,
                    height: lh,
                    depth_or_array_layers: 1,
                },
            );
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            view,
            size: [width, height],
        }
    }

    /// 包装一张已有的 GPU 纹理视图作为背景（例如离屏渲染目标）。
    ///
    /// 与 [`Backdrop::from_rgba`] 不同，这里不创建纹理、不生成 mip：
    /// `blur > 0` 的 lod 采样依赖该视图自带的 mip 链，没有就退化为末级纹理。
    /// 每帧离屏渲染的场景请配合 [`LiveBackdrop`] 使用（它负责 GPU 生成 mip）。
    pub fn from_view(view: wgpu::TextureView, size: [u32; 2]) -> Self {
        Self { view, size }
    }

    /// 纹理视图（绑定给合成器）。
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// 第 0 级尺寸。
    pub fn size(&self) -> [u32; 2] {
        self.size
    }
}

/// 离屏实时背景：一块每帧可重绘的渲染目标纹理，供合成器当作背景采样。
///
/// 典型帧序（同一个 `CommandEncoder` 内）：
/// 1. 把应用自己的场景渲染进 [`LiveBackdrop::target_view`]（mip 0）；
/// 2. [`LiveBackdrop::generate_mips`] 在 GPU 上逐级降采样出 mip 链；
/// 3. 把 [`LiveBackdrop::backdrop`] 交给 [`Compositor::render`](crate::Compositor::render)。
///
/// 纹理是 sRGB 编码（默认 [`wgpu::TextureFormat::Rgba8UnormSrgb`]）：场景 pass
/// 输出 sRGB 编码的颜色，mip 采样时硬件自动线性化，与 [`Backdrop::from_rgba`]
/// 的 CPU 线性空间盒滤波语义一致（2:1 降采样时双线性采样恰为 2×2 盒滤波）。
pub struct LiveBackdrop {
    backdrop: Backdrop,
    target_view: wgpu::TextureView,
    /// 每级一个单 mip 视图，blit 链复用，避免每帧重建。
    mip_views: Vec<wgpu::TextureView>,
    blitter: wgpu::util::TextureBlitter,
    format: wgpu::TextureFormat,
    mip_level_count: u32,
}

impl LiveBackdrop {
    /// 创建 `Rgba8UnormSrgb` 的实时背景。
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        Self::with_format(device, width, height, wgpu::TextureFormat::Rgba8UnormSrgb)
    }

    /// 以指定 sRGB 编码格式创建实时背景（如需与交换链格式对齐可用 `Bgra8UnormSrgb`）。
    pub fn with_format(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        assert!(width > 0 && height > 0, "live backdrop must be non-empty");
        assert!(
            format.is_srgb(),
            "live backdrop format must be sRGB-encoded"
        );

        let mip_level_count = 1 + width.max(height).ilog2();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vitreo-live-backdrop"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        let backdrop_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("vitreo-live-backdrop-view"),
            ..Default::default()
        });
        let target_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("vitreo-live-backdrop-target"),
            format: Some(format),
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_mip_level: 0,
            mip_level_count: Some(1),
            base_array_layer: 0,
            array_layer_count: Some(1),
            aspect: wgpu::TextureAspect::All,
            usage: None,
        });
        let mip_views = (0..mip_level_count)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("vitreo-live-backdrop-mip"),
                    format: Some(format),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    base_array_layer: 0,
                    array_layer_count: Some(1),
                    aspect: wgpu::TextureAspect::All,
                    usage: None,
                })
            })
            .collect();

        Self {
            backdrop: Backdrop {
                view: backdrop_view,
                size: [width, height],
            },
            target_view,
            mip_views,
            blitter: wgpu::util::TextureBlitter::new(device, format),
            format,
            mip_level_count,
        }
    }

    /// mip-0 渲染目标视图：应用每帧把自己的场景画进来。
    pub fn target_view(&self) -> &wgpu::TextureView {
        &self.target_view
    }

    /// 作为背景交给合成器（完整 mip 链视图）。
    pub fn backdrop(&self) -> &Backdrop {
        &self.backdrop
    }

    /// 第 0 级尺寸。
    pub fn size(&self) -> [u32; 2] {
        self.backdrop.size()
    }

    /// 在 GPU 上逐级生成 mip 链（双线性 2:1 降采样，sRGB 视图保证线性空间）。
    ///
    /// 在场景渲染之后、合成之前调用，同一个 encoder 内即可。
    pub fn generate_mips(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        for level in 1..self.mip_level_count {
            self.blitter.copy(
                device,
                encoder,
                &self.mip_views[(level - 1) as usize],
                &self.mip_views[level as usize],
            );
        }
    }

    /// 尺寸变化时重建纹理（旧内容不保留，下一帧场景重绘）。
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.backdrop.size() == [width, height] {
            return;
        }
        *self = Self::with_format(device, width, height, self.format);
    }
}

fn align_256(n: u32) -> u32 {
    n.div_ceil(256) * 256
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let v = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (v * 255.0 + 0.5) as u8
}

fn downsample(data: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
    let nw = (w / 2).max(1);
    let nh = (h / 2).max(1);
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for dy in 0..nh {
        for dx in 0..nw {
            let x0 = (dx * 2).min(w.saturating_sub(1));
            let y0 = (dy * 2).min(h.saturating_sub(1));
            let x1 = (dx * 2 + 1).min(w.saturating_sub(1));
            let y1 = (dy * 2 + 1).min(h.saturating_sub(1));
            let mut acc = [0.0f32; 3];
            let mut count = 0.0;
            for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
                let i = ((y * w + x) * 4) as usize;
                acc[0] += srgb_to_linear(data[i]);
                acc[1] += srgb_to_linear(data[i + 1]);
                acc[2] += srgb_to_linear(data[i + 2]);
                count += 1.0;
            }
            let o = ((dy * nw + dx) * 4) as usize;
            out[o] = linear_to_srgb(acc[0] / count);
            out[o + 1] = linear_to_srgb(acc[1] / count);
            out[o + 2] = linear_to_srgb(acc[2] / count);
            out[o + 3] = 255;
        }
    }
    (out, nw, nh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsample_halves_dimensions() {
        let (data, w, h) = (vec![128u8; 64 * 64 * 4], 64, 64);
        let (out, nw, nh) = downsample(&data, w, h);
        assert_eq!((nw, nh), (32, 32));
        assert_eq!(out.len(), 32 * 32 * 4);
    }

    #[test]
    fn downsample_handles_odd_dimensions() {
        let (data, w, h) = (vec![200u8; 5 * 3 * 4], 5, 3);
        let (out, nw, nh) = downsample(&data, w, h);
        assert_eq!((nw, nh), (2, 1));
        assert_eq!(out.len(), 2 * 4);
    }

    #[test]
    fn srgb_roundtrip() {
        for c in [0u8, 1, 64, 128, 200, 254, 255] {
            let back = linear_to_srgb(srgb_to_linear(c));
            assert!(
                (back as i32 - c as i32).abs() <= 1,
                "c = {c}, back = {back}"
            );
        }
    }
}
