//! 程序化生成的演示背景：对角渐变 + 网格 + 柔光斑 + 暗角。
//! 网格线让折射位移与色散边缘清晰可见。
//!
//! 逐像素代价压到几次乘加：柔光斑的高斯可分离（`exp(-(dx²+dy²)) =
//! exp(-dx²)·exp(-dy²)`），按行/列预计算；linear→sRGB 查 LUT。
//! 1600×1000 约 160 万像素在数十毫秒内完成，不拖慢首帧。

/// sRGB 转换 LUT 项数（量化步长 1/4095，输出误差远小于 1/255）。
const SRGB_LUT_N: usize = 4096;

pub fn generate_backdrop(w: u32, h: u32) -> Vec<u8> {
    let (wf, hf) = (w as f32, h as f32);
    let mut out = vec![0u8; (w * h * 4) as usize];

    // 多停靠点渐变（线性空间）
    let stops: [(f32, [f32; 3]); 4] = [
        (0.00, [0.075, 0.043, 0.212]), // 靛蓝
        (0.42, [0.302, 0.118, 0.557]), // 紫
        (0.72, [0.878, 0.310, 0.353]), // 珊瑚
        (1.00, [0.973, 0.682, 0.373]), // 金
    ];

    let glow_a = [0.20 * wf, 0.24 * hf];
    let glow_b = [0.80 * wf, 0.78 * hf];
    let cell = 56.0f32;

    // 行/列预计算：高斯的 x、y 两半、网格掩码、渐变与暗角的坐标项。
    let (inv_a, inv_b) = (
        1.0 / (2.0 * (0.42 * wf * 0.5).powi(2)),
        1.0 / (2.0 * (0.46 * wf * 0.5).powi(2)),
    );
    let col_a: Vec<f32> = (0..w)
        .map(|x| (-(x as f32 - glow_a[0]).powi(2) * inv_a).exp())
        .collect();
    let row_a: Vec<f32> = (0..h)
        .map(|y| (-(y as f32 - glow_a[1]).powi(2) * inv_a).exp())
        .collect();
    let col_b: Vec<f32> = (0..w)
        .map(|x| (-(x as f32 - glow_b[0]).powi(2) * inv_b).exp())
        .collect();
    let row_b: Vec<f32> = (0..h)
        .map(|y| (-(y as f32 - glow_b[1]).powi(2) * inv_b).exp())
        .collect();

    let gx: Vec<f32> = (0..w).map(|x| grid_mask(x as f32, cell)).collect();
    let gy: Vec<f32> = (0..h).map(|y| grid_mask(y as f32, cell)).collect();

    // 渐变参数 t = 0.35·x/w + 0.65·y/h；暗角 d = √(dx²+dy²)/√2。
    let tx: Vec<f32> = (0..w).map(|x| x as f32 / wf * 0.35).collect();
    let ty: Vec<f32> = (0..h).map(|y| y as f32 / hf * 0.65).collect();
    let dx2: Vec<f32> = (0..w)
        .map(|x| {
            let dx = (x as f32 - wf * 0.5) / (wf * 0.5);
            dx * dx
        })
        .collect();
    let dy2: Vec<f32> = (0..h)
        .map(|y| {
            let dy = (y as f32 - hf * 0.5) / (hf * 0.5);
            dy * dy
        })
        .collect();

    let lut = build_srgb_lut();

    let mut i = 0usize;
    for y in 0..h as usize {
        let (ra, rb) = (row_a[y], row_b[y]);
        let (gyy, tyy, dyy) = (gy[y], ty[y], dy2[y]);
        for x in 0..w as usize {
            let t = (tx[x] + tyy).clamp(0.0, 1.0);
            let mut c = gradient(&stops, t);

            // 柔光斑（两个可分离高斯的乘积）
            let ga = col_a[x] * ra;
            let gb = col_b[x] * rb;
            c[0] += 0.35 * ga + 0.08 * gb;
            c[1] += 0.12 * ga + 0.25 * gb;
            c[2] += 0.22 * ga + 0.30 * gb;

            // 网格（混向白色）与暗角（乘性压暗）：(c·(1−line) + line)·v
            let line = 0.10 * gx[x].max(gyy);
            let d = (dx2[x] + dyy).sqrt() / std::f32::consts::SQRT_2;
            let v = 1.0 - 0.22 * smoothstep(0.55, 1.0, d);
            let scale = (1.0 - line) * v;
            let line_v = line * v;

            out[i] = srgb_lut(&lut, c[0] * scale + line_v);
            out[i + 1] = srgb_lut(&lut, c[1] * scale + line_v);
            out[i + 2] = srgb_lut(&lut, c[2] * scale + line_v);
            out[i + 3] = 255;
            i += 4;
        }
    }
    out
}

fn build_srgb_lut() -> [u8; SRGB_LUT_N] {
    let mut lut = [0u8; SRGB_LUT_N];
    for (i, v) in lut.iter_mut().enumerate() {
        *v = linear_to_srgb(i as f32 / (SRGB_LUT_N - 1) as f32);
    }
    lut
}

#[inline]
fn srgb_lut(lut: &[u8; SRGB_LUT_N], c: f32) -> u8 {
    lut[(c.clamp(0.0, 1.0) * (SRGB_LUT_N - 1) as f32 + 0.5) as usize]
}

fn gradient(stops: &[(f32, [f32; 3])], t: f32) -> [f32; 3] {
    if t <= stops[0].0 {
        return stops[0].1;
    }
    for pair in stops.windows(2) {
        let (t0, c0) = pair[0];
        let (t1, c1) = pair[1];
        if t <= t1 {
            let k = (t - t0) / (t1 - t0);
            return [
                c0[0] + (c1[0] - c0[0]) * k,
                c0[1] + (c1[1] - c0[1]) * k,
                c0[2] + (c1[2] - c0[2]) * k,
            ];
        }
    }
    stops[stops.len() - 1].1
}

fn grid_mask(v: f32, cell: f32) -> f32 {
    let m = v.rem_euclid(cell);
    let d = m.min(cell - m);
    1.0 - smoothstep(0.5, 1.5, d)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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
