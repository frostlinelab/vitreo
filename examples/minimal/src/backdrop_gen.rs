//! 程序化生成的演示背景：对角渐变 + 网格 + 柔光斑 + 暗角。
//! 网格线让折射位移与色散边缘清晰可见。

pub fn generate_backdrop(w: u32, h: u32) -> Vec<u8> {
    let (wf, hf) = (w as f32, h as f32);
    let mut out = Vec::with_capacity((w * h * 4) as usize);

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

    for y in 0..h {
        for x in 0..w {
            let (xf, yf) = (x as f32, y as f32);

            let t = ((xf / wf) * 0.35 + (yf / hf) * 0.65).clamp(0.0, 1.0);
            let mut c = gradient(&stops, t);

            // 柔光斑
            c[0] += 0.35 * gauss(xf, yf, glow_a, 0.42 * wf);
            c[1] += 0.12 * gauss(xf, yf, glow_a, 0.42 * wf);
            c[2] += 0.22 * gauss(xf, yf, glow_a, 0.42 * wf);
            c[0] += 0.08 * gauss(xf, yf, glow_b, 0.46 * wf);
            c[1] += 0.25 * gauss(xf, yf, glow_b, 0.46 * wf);
            c[2] += 0.30 * gauss(xf, yf, glow_b, 0.46 * wf);

            // 网格
            let g = grid_mask(xf, cell).max(grid_mask(yf, cell));
            let line = 0.10 * g;
            c = [
                c[0] * (1.0 - line) + line,
                c[1] * (1.0 - line) + line,
                c[2] * (1.0 - line) + line,
            ];

            // 暗角
            let dx = (xf - wf * 0.5) / (wf * 0.5);
            let dy = (yf - hf * 0.5) / (hf * 0.5);
            let d = (dx * dx + dy * dy).sqrt() / std::f32::consts::SQRT_2;
            let v = 1.0 - 0.22 * smoothstep(0.55, 1.0, d);
            c = [c[0] * v, c[1] * v, c[2] * v];

            out.push(linear_to_srgb(c[0]));
            out.push(linear_to_srgb(c[1]));
            out.push(linear_to_srgb(c[2]));
            out.push(255);
        }
    }
    out
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

fn gauss(x: f32, y: f32, center: [f32; 2], radius: f32) -> f32 {
    let sigma = radius * 0.5;
    let dx = x - center[0];
    let dy = y - center[1];
    (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp()
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
