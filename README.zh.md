# Vitreo

> **Write once, refract everywhere.（折射写一次，处处皆玻璃。）**
> 立项 2026-09-25 · MIT · 状态：P3 完成，P4 待启动

渲染器无关的 Rust 玻璃材质层，基于 wgpu：单次片元着色完成 SDF 折射、色散与 Fresnel 边缘辉光。任何应用——egui、游戏、编辑器、overlay——都能给任意区域加上 Apple **Liquid Glass** 级别的玻璃材质。英文文档：[README.md](README.md)

一句话设计准则：**光学的部分跨平台共享，身份的部分每平台本地化。折射写一次，铬（chrome）各写各的。**

## 为什么到处都成立

玻璃不是"皮肤"，而是**光学介质**：视觉内容 100% 来自背后正在发生的内容，自身不携带任何平台身份，所以放在 macOS、Windows 还是 Hyprland 上都相适相融。透明 + 从背景取光已是三方生态的公共底色（macOS vibrancy、Windows Mica/Acrylic、Hyprland/KWin blur）；Vitreo 把这份共有审美从*磨砂*升级到*折射*——从物理算起，不逆向任何 Apple 接口。

## 技术管线

`sdRoundedBox` SDF → bevel 边缘法线 → `refract()` 屏幕空间位移 → 每通道独立色散 → Schlick–Fresnel 边缘高光。IOR 是真实材料常数（默认 ≈ 1.52，冕牌玻璃）。每条公式的推导见 [**Vitreo 的光学**](docs/optics.zh.md)（[英文原文](docs/optics.en.md)），全部插图由着色器数学的精确复现生成。

## 架构

```
vitreo/            # 纯 wgpu：材质 + 合成器 + 动画层，零 UI 依赖
├─ glass.wgsl      #   折射 shader（含加速度/按压驱动的果冻形变）
├─ GlassStyle      #   IOR、thickness、depth、bevel、blur、tint、dispersion
├─ Backdrop        #   静态纹理 / 离屏实时纹理 / 视频帧
├─ Compositor      #   多面板合成、重叠 Merge/Stack 策略
├─ GlassLayer      #   弹簧面板、拖拽/DPI 处理、LiveBackdrop 离屏管线
└─ GlassButton     #   胶囊玻璃按钮 —— 悬停/按压/点击（第一个控件）
vitreo-egui        # egui 绑定：EguiFrame 帧接线 + 系统 CJK 字体
examples/
├─ minimal         # 渐变/图片背景 + 可拖玻璃面板 ✓
├─ live-backdrop   # 折射应用自己渲染的场景：LiveBackdrop + Merge ✓
└─ tuner           # 参数游乐场 —— 计划中（P3+）
```

## 快速开始

Rust 工具链由 `rust-toolchain.toml` 钉死；任何 wgpu 支持的 GPU（Vulkan / Metal / DX12）均可。

```sh
cargo run -p minimal
```

会打开一个渐变背景 + 可拖拽玻璃面板的窗口。拖拽带弹簧果冻手感——轮廓沿运动方向拉伸、折射滞后于手势（见[光学报告 §7](docs/optics.zh.md)）。底部有一颗胶囊玻璃按钮：按下微缩、松开带果冻回弹，点击计数显示在参数面板里。把图片拖进窗口（或传 `--image 路径/到/图片.png`，或用文件选择器）即可折射你自己的图片；egui 面板实时调节 IOR、bevel、色散与 Fresnel。

```sh
cargo run -p live-backdrop
```

P2 验收 demo：玻璃折射的是示例**自己渲染**的动画场景（aurora + 网格）。两块面板初始故意重叠——`M` 键在 Stack（逐层覆盖）与 Merge（并集融合成一块连续玻璃）之间切换。

## 路线图

- [x] **P0** — 把 jeantimex 的 glass shader 抠成无 DOM 依赖的纯纹理输入版
- [x] **P1** — minimal demo：SDF 圆角面板、拖拽、折射 + 色散 + Fresnel 全开（核心验收）
- [x] **P2** — 离屏 live-backdrop 管线 + 多面板合成器（Stack/Merge）
- [x] **P3** — `vitreo-egui` 绑定 crate：弹簧面板 + shader 级果冻形变（速度挤压拉伸、法线滞后、按压微缩——见光学报告 §7）
- [ ] **P4** — 发布 `vitreo` 0.1 + showcase

## 文档

- [Vitreo 的光学](docs/optics.zh.md) / [The Optics of Vitreo](docs/optics.en.md) — 物理报告：每条公式、每个常数、每张图都从源码推导
- [docs/figures/generate.py](docs/figures/generate.py) — 双语图像管线；用 NumPy 重跑着色器数学（对拍基准）
- [AGENTS.md](AGENTS.md) — 给 AI 协作者的工作约定（对拍不变量、提交规范、许可证规则）

## 许可证与致谢

Vitreo 以 [MIT](LICENSE) 许可发布。站在这些 MIT 项目的肩膀上——致谢：

- [jeantimex/glass-effect-webgpu](https://github.com/jeantimex/glass-effect-webgpu) — WGSL 折射 shader 谱系（thickness/bezel/refraction/blur/specular uniform 管线），README credit
- [m2-md/liquid-glass-refraction-shader](https://github.com/m2-md/liquid-glass-refraction-shader) — IOR、色散、Fresnel 数学正源；optics.ts / sdf2d.ts + 84 个测试翻成 Rust 单测
- [heonny/egui-glass](https://github.com/heonny/egui-glass) — egui-wgpu LiveBackdrop 离屏渲染骨架参考
- [charlie-x/liquidImgui 及其 forks](https://github.com/charlie-x/liquidImgui) — Windows DXGI Desktop Duplication 桌面合成参考
- [zaroutt/Niri-glass](https://github.com/zaroutt/Niri-glass) — Wayland 合成器级 SDF 折射（GLSL）
- [OverShifted/LiquidGlass](https://github.com/OverShifted/LiquidGlass) — C++/OpenGL SDF 折射参考
