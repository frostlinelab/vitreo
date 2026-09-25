# Vitreo UI：跨平台 Rust 玻璃 UI 框架

> 立项日期：2026-09-25 · 状态：P0 启动
Tagline：**Write once, refract everywhere.**
> 

## 定位

渲染器无关的 Rust 玻璃材质层：一个基于 wgpu 的光学核心（SDF 折射、色散、Fresnel），任何应用——egui、游戏、编辑器、overlay——都能给任意区域加上 Apple Liquid Glass 级别的玻璃材质。

一句话设计准则：**光学的部分跨平台共享，身份的部分每平台本地化。折射写一次，铬各写各的。**

## 为什么成立

玻璃不是"皮肤"，是"光学介质"：视觉内容 100% 来自背后正在发生的内容，不携带平台身份，所以放在 macOS / Windows / Hyprland 上都相适相融。透明 + 从背景取光已是三方公共底色（macOS vibrancy、Windows Mica/Acrylic、Hyprland/KWin blur），本项目是把共有审美从"磨砂"升级到"折射"。

## 技术管线（从物理算起，不逆向任何 Apple 接口）

`sdRoundedBox` SDF → bevel 边缘算法线 → `refract()` 屏幕空间偏移 → 每通道独立 eta 色散 → Schlick–Fresnel 边缘高光。IOR 是真实材料常数（默认 ≈1.52）。

## 架构

```
vitreo/
├─ vitreo      # 纯 wgpu：材质 + 合成器，零 UI 依赖
│   ├─ glass.wgsl           # 折射 shader
│   ├─ GlassStyle           # IOR、thickness、bezel、blur、tint、dispersion
│   ├─ Backdrop             # 静态纹理 / 离屏实时纹理 / 视频帧
│   └─ Compositor           # 多面板合成、重叠 Merge/Stack 策略
├─ glasses-egui      # egui 绑定（LiveBackdrop 离屏管线）
└─ examples/
    ├─ minimal             # 渐变背景 + 可拖玻璃面板
    ├─ live-backdrop       # 折射自己渲染的场景
    └─ tuner               # 参数面板（移植 jeantimex 的调试 UI）
```

## 设计准则

- 玻璃 = 介质，不做固定纹理
- 光学跨平台共享；corner radius、tint、控件形状做成 per-platform token
- MIT 协议；致谢所有来源项目

## Roadmap

- [ ]  **P0** — 把 jeantimex 的 glass shader 抠成无 DOM 依赖的纯纹理输入版
- [ ]  **P1** — minimal demo：SDF 圆角面板、拖拽、折射 + 色散 + Fresnel 全开（核心验收）
- [ ]  **P2** — 离屏 live backdrop 管线 + 多面板合成器
- [ ]  **P3** — egui 绑定 crate + 弹簧形变动画
- [ ]  **P4** — 发布 `vitreo` 0.1 + showcase

## 致谢与参考

- [jeantimex/glass-effect-webgpu](https://github.com/jeantimex/glass-effect-webgpu) — MIT；WGSL 折射 shader 直接来源（thickness/bezel/refraction/blur/specular uniform 管线），README credit
- [m2-md/liquid-glass-refraction-shader](https://github.com/m2-md/liquid-glass-refraction-shader) — IOR、色散、Fresnel 数学正源；optics.ts / sdf2d.ts + 84 个测试翻成 Rust 单测
- [heonny/egui-glass](https://github.com/heonny/egui-glass) — egui-wgpu LiveBackdrop 离屏渲染骨架参考
- [liquidDX11 及其 forks](https://github.com/charlie-x/liquidImgui) — Windows DXGI Desktop Duplication 桌面合成参考
- [zaroutt/Niri-glass](https://github.com/zaroutt/Niri-glass) — Wayland 合成器级 SDF 折射（GLSL）
- [OverShifted/LiquidGlass](https://github.com/OverShifted/LiquidGlass) — C++/OpenGL SDF 折射参考

> 下一步：P0 动手时，先 clone jeantimex 仓库抠 `src/webgpu` 下的 WGSL，对照 m2-md 的 optics 数学去 DOM 化。
>