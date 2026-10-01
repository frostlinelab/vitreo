// glass.wgsl — Vitreo 核心合成 pass。
//
// 输入是纯纹理背景（无 DOM 依赖），输出背景 + 任意数量玻璃面板的一次性合成。
// 单面板光学管线：
//   sdRoundedBox SDF → bevel 高度场 → 法线 → refract() 屏幕空间位移 →
//   每通道独立 eta 色散 → Schlick–Fresnel 边缘辉光 + 镜面高光。
//
// 数学移植自 m2-md/liquid-glass-refraction-shader（MIT）；
// 多面板合成结构参考 jeantimex/glass-effect-webgpu（MIT）。

const MAX_PANELS: u32 = 8u;
const SPECTRAL_SAMPLES: u32 = 3u;
const NORMAL_EPS: f32 = 1.0;

// 合成策略（与 Rust 侧 CompositeStrategy 的判别值一致）。
const STRATEGY_STACK: u32 = 0u;
const STRATEGY_MERGE: u32 = 1u;

struct Globals {
    viewport: vec2f,   // 物理像素
    time: f32,         // 秒
    panel_count: u32,
    strategy: u32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,        // uniform 地址空间要求 struct 大小为 16 的倍数
}

// 面板 uniform（96 字节，与 Rust 侧 PanelUniform 逐字节一致；不用 vec3 占位，
// 避免 uniform 地址空间的 16 字节对齐自动填充。velocity 按两个标量声明，
// 与 Rust 侧 [f32;2] 逐字节一致且不破坏 tint/shadow 的 16 对齐）。
struct PanelData {
    center: vec2f,
    half_size: vec2f,
    corner_radius: f32,
    bevel: f32,
    thickness: f32,
    depth: f32,
    ior: f32,
    dispersion: f32,
    blur: f32,
    specular: f32,
    fresnel_f0: f32,
    velocity_x: f32,  // 果冻形变：运动速度 x（像素/秒）
    velocity_y: f32,  // 果冻形变：运动速度 y
    press: f32,       // 果冻形变：按压量 0..1
    tint: vec4f,      // rgb = 染色/辉光色，a = 染色不透明度
    shadow: vec4f,    // x = 不透明度, y = 模糊半径, zw = 偏移
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<uniform> panels: array<PanelData, 8>;
@group(0) @binding(2) var backdrop: texture_2d<f32>;
@group(0) @binding(3) var backdrop_sampler: sampler;

// ---------- SDF 与表面几何（与 sdf2d.rs 对应） ----------

fn sd_rounded_box(p: vec2f, b: vec2f, r: f32) -> f32 {
    let q = abs(p) - b + vec2f(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2f(0.0))) - r;
}

fn glass_height(d: f32, w: f32) -> f32 {
    let x = clamp(-d / max(w, 1e-3), 0.0, 1.0);
    let t = 1.0 - x;
    return sqrt(max(1.0 - t * t, 0.0));
}

fn glass_normal(p: vec2f, half_size: vec2f, radius: f32, bevel: f32, thickness: f32) -> vec3f {
    let ex = vec2f(NORMAL_EPS, 0.0);
    let ey = vec2f(0.0, NORMAL_EPS);
    let hx = glass_height(sd_rounded_box(p + ex, half_size, radius), bevel)
           - glass_height(sd_rounded_box(p - ex, half_size, radius), bevel);
    let hy = glass_height(sd_rounded_box(p + ey, half_size, radius), bevel)
           - glass_height(sd_rounded_box(p - ey, half_size, radius), bevel);
    let k = thickness / (2.0 * NORMAL_EPS);
    return normalize(vec3f(-hx * k, -hy * k, 1.0));
}

// ---------- 果冻形变（与 sdf2d.rs 的 JELLY_* / jelly_* 逐行对应） ----------
//
// 艺术性扩展（squash & stretch 动画原理）：运动中的面板沿速度方向伸长、
// 垂直方向收缩，按压时整体微缩，bevel 法线滞后于运动。

const JELLY_STRETCH_GAIN: f32 = 4e-4;  // e = |velocity| * GAIN
const JELLY_STRETCH_MAX: f32 = 0.15;   // 拉伸量上限
const JELLY_PRESS_SQUASH: f32 = 0.05;  // press = 1 时整体缩到 95%
const JELLY_NORMAL_LAG: f32 = 0.9;     // 法线滞后倾斜增益

// velocity_x/y 以标量存储（对齐考虑），组装成 vec2f 使用。
fn panel_velocity(panel: PanelData) -> vec2f {
    return vec2f(panel.velocity_x, panel.velocity_y);
}

// 把局部坐标 p 映射进未变形盒子的采样空间：先按压均匀微缩，
// 再沿速度方向压缩采样坐标（旋转到速度系 → 各向异性缩放 → 旋转回来，
// 盒子半尺寸是轴对齐的，不能省略最后的回转）。
fn jelly_sample(p: vec2f, velocity: vec2f, press: f32) -> vec2f {
    let k = 1.0 - JELLY_PRESS_SQUASH * press;
    var q = p / k;
    let speed = length(velocity);
    if (speed > 1e-3) {
        let dir = velocity / speed;
        let e = min(speed * JELLY_STRETCH_GAIN, JELLY_STRETCH_MAX);
        let along = dot(q, dir);
        let perp = dot(q, vec2f(-dir.y, dir.x));
        let a = along / (1.0 + e);
        let b = perp / (1.0 - 0.5 * e);
        q = vec2f(dir.x * a - dir.y * b, dir.y * a + dir.x * b);
    }
    return q;
}

// 变形 SDF 的距离修正系数：按压缩放 × 拉伸的垂直收缩量。
// 零点（轮廓位置）不受影响，只修正梯度量级以保持 AA 带宽近似不变。
fn jelly_scale(velocity: vec2f, press: f32) -> f32 {
    let k = 1.0 - JELLY_PRESS_SQUASH * press;
    let e = min(length(velocity) * JELLY_STRETCH_GAIN, JELLY_STRETCH_MAX);
    return k * (1.0 - 0.5 * e);
}

// 变形后的圆角盒 SDF：sd_rounded_box 的果冻版本。
fn jelly_sdf(p: vec2f, half_size: vec2f, radius: f32, velocity: vec2f, press: f32) -> f32 {
    return sd_rounded_box(jelly_sample(p, velocity, press), half_size, radius)
         * jelly_scale(velocity, press);
}

// 法线滞后倾斜：在 bevel 法线上叠加与速度反向的倾斜（玻璃"跟不上"运动）。
// d 是该像素的变形后 SDF，用于边缘权重——只倾斜 bevel 带，内部平台保持平坦。
fn jelly_lag(n: vec3f, velocity: vec2f, d: f32, bevel: f32) -> vec3f {
    let speed = length(velocity);
    if (speed < 1e-3) {
        return n;
    }
    let dir = velocity / speed;
    let e = min(speed * JELLY_STRETCH_GAIN, JELLY_STRETCH_MAX);
    let w = (1.0 - glass_height(d, bevel)) * JELLY_NORMAL_LAG;
    return normalize(vec3f(n.xy - dir * (e * w), n.z));
}

// ---------- 光学（与 optics.rs 对应） ----------

fn refract_offset(n: vec3f, eta: f32, depth: f32) -> vec2f {
    // 视线垂直入射屏幕：I = (0, 0, -1)，n 朝向观察者。
    let r = refract(vec3f(0.0, 0.0, -1.0), n, eta);
    if (dot(r, r) < 0.5) {
        return vec2f(0.0); // 全内反射
    }
    return r.xy * (depth / max(abs(r.z), 1e-3));
}

fn fresnel_schlick(cos_theta: f32, f0: f32) -> f32 {
    let c = clamp(1.0 - cos_theta, 0.0, 1.0);
    return f0 + (1.0 - f0) * pow(c, 5.0);
}

// t: 0 红端, 1 蓝端。粗粒度的三峰光谱权重。
fn spectrum_weight(t: f32) -> vec3f {
    return vec3f(
        exp(-16.0 * (t - 0.15) * (t - 0.15)),
        exp(-16.0 * (t - 0.50) * (t - 0.50)),
        exp(-16.0 * (t - 0.85) * (t - 0.85)),
    );
}

// ---------- 背景采样 ----------

fn sample_backdrop(pixel: vec2f, lod: f32) -> vec3f {
    let uv = pixel / globals.viewport;
    let half_texel = vec2f(0.5) / vec2f(textureDimensions(backdrop));
    let clamped = clamp(uv, half_texel, vec2f(1.0) - half_texel);
    return textureSampleLevel(backdrop, backdrop_sampler, clamped, lod).rgb;
}

fn blur_lod(blur_px: f32) -> f32 {
    return clamp(log2(max(blur_px, 1.0)), 0.0, 12.0);
}

// 光谱折射：每通道独立 eta（SPECTRAL_SAMPLES 个采样点线性扫过 dispersion 范围，
// 各通道除以自己的权重和，白仍是白）。
fn refract_backdrop(pixel: vec2f, n: vec3f, panel: PanelData, lod: f32) -> vec3f {
    if (panel.dispersion <= 0.0) {
        let off = refract_offset(n, 1.0 / panel.ior, panel.depth);
        return sample_backdrop(pixel + off, lod);
    }

    var sum = vec3f(0.0);
    var wsum = vec3f(0.0);
    for (var i = 0u; i < SPECTRAL_SAMPLES; i = i + 1u) {
        let t = f32(i) / f32(SPECTRAL_SAMPLES - 1u);
        let ni = panel.ior + panel.dispersion * (t - 0.5); // 蓝端折射率更高
        let off = refract_offset(n, 1.0 / ni, panel.depth);
        let w = spectrum_weight(t);
        sum = sum + sample_backdrop(pixel + off, lod) * w;
        wsum = wsum + w;
    }
    return sum / max(wsum, vec3f(1e-4));
}

// ---------- 单面板着色 ----------

// 给定表面法线，完成折射/色散 → Fresnel 辉光 → 镜面高光 → 染色。
// 单面板（Stack）与并集融合（Merge）共用这条着色路径。
fn shade_glass_pixel(pixel: vec2f, n: vec3f, panel: PanelData) -> vec3f {
    let refr = refract_backdrop(pixel, n, panel, blur_lod(panel.blur));

    // Schlick–Fresnel：掠射角（边缘）处混入辉光色。
    let view = vec3f(0.0, 0.0, 1.0);
    let f = fresnel_schlick(max(dot(n, view), 0.0), panel.fresnel_f0);
    let sky = panel.tint.rgb * (0.55 - 0.45 * n.y); // y 向下坐标系，光自上方来
    var col = mix(refr, sky, f);

    // 镜面高光（m2-md 的主光方向，y 翻转到 y 向下的屏幕坐标）。
    let l = normalize(vec3f(-0.35, -0.72, 0.60));
    let spec = pow(max(dot(reflect(-view, n), l), 0.0), 48.0);
    col = col + vec3f(1.0, 0.98, 0.94) * spec * panel.specular;

    // 可选染色。
    return mix(col, panel.tint.rgb, panel.tint.a);
}

fn shade_glass(pixel: vec2f, panel: PanelData, d: f32) -> vec3f {
    // 法线在变形采样空间求值，再叠加滞后倾斜；d 是该像素的变形后 SDF。
    let p = jelly_sample(pixel - panel.center, panel_velocity(panel), panel.press);
    var n = glass_normal(p, panel.half_size, panel.corner_radius, panel.bevel, panel.thickness);
    n = jelly_lag(n, panel_velocity(panel), d, panel.bevel);
    return shade_glass_pixel(pixel, n, panel);
}

// ---------- 多面板：Merge（并集融合，与 sdf2d.rs 的并集法线对应） ----------

// 所有面板的并集 SDF：内部为负。重叠区的融合几何全部来自它。
// 每个面板贡献自己的果冻变形 SDF。
fn union_sdf(pixel: vec2f, count: u32) -> f32 {
    var d = 1e30;
    for (var i = 0u; i < count; i = i + 1u) {
        let panel = panels[i];
        d = min(d, jelly_sdf(pixel - panel.center, panel.half_size, panel.corner_radius,
                             panel_velocity(panel), panel.press));
    }
    return d;
}

// 并集高度场的有限差分法线：倒角只出现在并集外轮廓，
// 重叠内部没有内边缘 —— 这就是"融合成一块玻璃"的几何来源。
fn union_normal(pixel: vec2f, count: u32, bevel: f32, thickness: f32) -> vec3f {
    let ex = vec2f(NORMAL_EPS, 0.0);
    let ey = vec2f(0.0, NORMAL_EPS);
    let hx = glass_height(union_sdf(pixel + ex, count), bevel)
           - glass_height(union_sdf(pixel - ex, count), bevel);
    let hy = glass_height(union_sdf(pixel + ey, count), bevel)
           - glass_height(union_sdf(pixel - ey, count), bevel);
    let k = thickness / (2.0 * NORMAL_EPS);
    return normalize(vec3f(-hx * k, -hy * k, 1.0));
}

// 材质参数取覆盖该像素的最上层面板（数组末尾最上层）；
// 抗锯齿边缘带内没有面板严格包含像素，退回取离并集边缘最近者。
fn merge_material(pixel: vec2f, count: u32) -> PanelData {
    for (var i = count; i > 0u; i = i - 1u) {
        let panel = panels[i - 1u];
        let d = jelly_sdf(pixel - panel.center, panel.half_size, panel.corner_radius,
                          panel_velocity(panel), panel.press);
        if (d < 0.0) {
            return panel;
        }
    }
    var mat = panels[0];
    var nearest = 1e30;
    for (var i = 0u; i < count; i = i + 1u) {
        let panel = panels[i];
        let d = jelly_sdf(pixel - panel.center, panel.half_size, panel.corner_radius,
                          panel_velocity(panel), panel.press);
        if (d < nearest) {
            nearest = d;
            mat = panel;
        }
    }
    return mat;
}

fn shade_merged(pixel: vec2f, count: u32) -> vec3f {
    let mat = merge_material(pixel, count);
    let d = union_sdf(pixel, count);
    var n = union_normal(pixel, count, mat.bevel, mat.thickness);
    // 滞后倾斜取最上层面板的速度；并集轮廓已是变形后的形状。
    n = jelly_lag(n, panel_velocity(mat), d, mat.bevel);
    return shade_glass_pixel(pixel, n, mat);
}

// 并集外的软阴影：各面板按自己的 shadow 参数在自身（变形后）SDF 上贡献，
// 玻璃覆盖处（并集内）不投阴影。
fn merged_shadow(pixel: vec2f, count: u32, color: vec3f) -> vec3f {
    var c = color;
    for (var i = 0u; i < count; i = i + 1u) {
        let panel = panels[i];
        if (panel.shadow.x <= 0.0) {
            continue;
        }
        let sp = pixel - panel.center - panel.shadow.zw;
        let sd = jelly_sdf(sp, panel.half_size, panel.corner_radius,
                           panel_velocity(panel), panel.press);
        let blur = max(panel.shadow.y, 1.0);
        let a = smoothstep(-blur, blur * 0.5, -sd) * panel.shadow.x;
        c = mix(c, vec3f(0.0), a);
    }
    return c;
}

// ---------- 全屏 pass ----------

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

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4f {
    let pixel = input.position.xy;
    var color = sample_backdrop(pixel, 0.0);

    let count = min(globals.panel_count, MAX_PANELS);
    if (globals.strategy == STRATEGY_MERGE && count > 0u) {
        // Merge：所有面板按并集 SDF 融合成一块连续玻璃。
        let d = union_sdf(pixel, count);
        let coverage = 1.0 - smoothstep(-1.0, 1.0, d); // 1px 抗锯齿边缘
        if (coverage > 0.0) {
            let shaded = shade_merged(pixel, count);
            color = mix(color, shaded, coverage);
        } else {
            color = merged_shadow(pixel, count, color);
        }
    } else {
        // Stack：按数组顺序逐层覆盖，下标越靠后越靠上层。
        for (var i = 0u; i < count; i = i + 1u) {
            let panel = panels[i];
            let d = jelly_sdf(pixel - panel.center, panel.half_size, panel.corner_radius,
                              panel_velocity(panel), panel.press);
            let coverage = 1.0 - smoothstep(-1.0, 1.0, d); // 1px 抗锯齿边缘

            if (coverage > 0.0) {
                let shaded = shade_glass(pixel, panel, d);
                color = mix(color, shaded, coverage);
            } else if (panel.shadow.x > 0.0) {
                // 面板外：SDF 软阴影（随果冻形变一起变形）。
                let sp = pixel - panel.center - panel.shadow.zw;
                let sd = jelly_sdf(sp, panel.half_size, panel.corner_radius,
                                   panel_velocity(panel), panel.press);
                let blur = max(panel.shadow.y, 1.0);
                let a = smoothstep(-blur, blur * 0.5, -sd) * panel.shadow.x;
                color = mix(color, vec3f(0.0), a);
            }
        }
    }

    return vec4f(color, 1.0);
}
