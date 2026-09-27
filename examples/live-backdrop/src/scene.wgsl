// scene.wgsl — live-backdrop 示例的离屏动画场景。
//
// 渲染进 LiveBackdrop 的 mip-0 目标，玻璃面板把它当作"应用自己的画面"来折射。
// 内容特意挑了三类折射敏感的元素：
//   1. 缓慢流动的 aurora 带（大面积渐变，看色散的彩色镶边）；
//   2. 两颗游走的高斯光斑（看大位移处的扭曲）；
//   3. 细网格 + 粗网格（直线一弯就知道位移场长什么样）。
//
// 全程线性光计算，sRGB 编码由 Rgba8UnormSrgb 目标格式负责。

struct SceneGlobals {
    viewport: vec2f, // 物理像素
    time: f32,       // 秒
    speed: f32,      // 场景时间倍率（暂停 = 0）
}

@group(0) @binding(0) var<uniform> g: SceneGlobals;

struct VertexOut {
    @builtin(position) position: vec4f,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    let pos = array(
        vec2f(-1.0,  1.0),
        vec2f(-1.0, -1.0),
        vec2f( 1.0,  1.0),
        vec2f( 1.0, -1.0),
    );
    var out: VertexOut;
    out.position = vec4f(pos[vertex_index], 0.0, 1.0);
    return out;
}

fn band_color(k: u32) -> vec3f {
    // aurora 三色：青 / 绿 / 紫罗兰
    if (k == 0u) { return vec3f(0.05, 0.55, 0.60); }
    if (k == 1u) { return vec3f(0.10, 0.75, 0.35); }
    return vec3f(0.45, 0.20, 0.75);
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4f {
    let t = g.time * g.speed;
    let uv = input.position.xy / g.viewport;
    var col = vec3f(0.012, 0.020, 0.050); // 深夜底色（线性）

    // ---- aurora 带：三条相位错开的正弦带，高斯截面 ----
    for (var k = 0u; k < 3u; k = k + 1u) {
        let fk = f32(k);
        let y0 = 0.28 + 0.10 * fk
               + 0.13 * sin(t * (0.21 + 0.06 * fk) + fk * 2.1)
               + 0.05 * sin(uv.x * (5.0 + 2.0 * fk) + t * (0.35 + 0.10 * fk) + fk * 1.7);
        let d = uv.y - y0;
        let w = 0.045 + 0.012 * sin(uv.x * 3.0 + t * 0.2 + fk);
        let band = exp(-d * d / (2.0 * w * w));
        col += band_color(k) * band * (0.55 - 0.10 * fk);
    }

    // ---- 两颗游走光斑 ----
    let c1 = vec2f(0.34 + 0.20 * sin(t * 0.31 + 1.0), 0.44 + 0.24 * cos(t * 0.23));
    let c2 = vec2f(0.70 + 0.16 * cos(t * 0.19), 0.58 + 0.20 * sin(t * 0.27 + 2.0));
    let d1 = length((uv - c1) * vec2f(g.viewport.x / g.viewport.y, 1.0));
    let d2 = length((uv - c2) * vec2f(g.viewport.x / g.viewport.y, 1.0));
    col += vec3f(0.90, 0.55, 0.20) * exp(-d1 * d1 / 0.010) * 0.9;
    col += vec3f(0.20, 0.45, 1.00) * exp(-d2 * d2 / 0.014) * 0.9;

    // ---- 网格：细格 48px、粗格 240px，直线弯曲就是位移场本身 ----
    let p = input.position.xy;
    let fine = abs(fract(p / 48.0) - 0.5);
    let coarse = abs(fract(p / 240.0) - 0.5);
    let fine_line = smoothstep(0.47, 0.5, max(fine.x, fine.y));
    let coarse_line = smoothstep(0.475, 0.5, max(coarse.x, coarse.y));
    col += vec3f(0.25, 0.32, 0.40) * fine_line * 0.20;
    col += vec3f(0.55, 0.65, 0.75) * coarse_line * 0.30;

    // ---- 暗角 ----
    let r = length(uv - vec2f(0.5)) * 1.35;
    col *= 1.0 - 0.5 * r * r;

    return vec4f(col, 1.0);
}
