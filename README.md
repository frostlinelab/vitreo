# Vitreo

> **Write once, refract everywhere.**
> Founded 2026-09-25 · MIT · status: P3 done, P4 next

A renderer-agnostic Rust glass-material layer built on wgpu: SDF refraction, chromatic dispersion, and Fresnel edge glow in one fragment pass. Any application — egui, games, editors, overlays — can give any region an Apple **Liquid Glass**-grade glass material. 中文说明：[README.zh.md](README.zh.md)

One-line design rule: **the optics are shared cross-platform, the identity is localized per platform. Refraction is written once; the chrome is written per platform.**

## Why this works everywhere

Glass is not a "skin" — it is an **optical medium**: 100% of its visual content comes from what is happening behind it, so it carries no platform identity of its own and sits naturally on macOS, Windows, or Hyprland alike. Transparency plus taking light from the backdrop is already the common ground of the three ecosystems (macOS vibrancy, Windows Mica/Acrylic, Hyprland/KWin blur); Vitreo upgrades that shared aesthetic from *frosted* to *refracted* — computed from physics, reverse-engineering no Apple interface.

## Pipeline

`sdRoundedBox` SDF → bevel edge normals → `refract()` screen-space displacement → per-channel dispersion → Schlick–Fresnel edge highlight. IOR is a real material constant (default ≈ 1.52, crown glass). Every equation is derived in [**The Optics of Vitreo**](docs/optics.en.md) ([中文版](docs/optics.zh.md)), with figures generated from the exact shader math.

## Architecture

```
vitreo/            # pure wgpu: material + compositor + animation layer, zero UI deps
├─ glass.wgsl      #   the refraction shader (incl. acceleration/press jelly deformation)
├─ GlassStyle      #   IOR, thickness, depth, bevel, blur, tint, dispersion
├─ Backdrop        #   static texture / offscreen live texture / video frame
├─ Compositor      #   multi-panel compositing, overlap Merge/Stack policies
└─ GlassLayer      #   spring panels, drag/DPI handling, LiveBackdrop pipeline
vitreo-egui        # egui bindings: EguiFrame render glue + system CJK fonts
examples/
├─ minimal         # gradient/image backdrop + draggable glass panel ✓
├─ live-backdrop   # refracting the app's own rendered scene: LiveBackdrop + Merge ✓
└─ tuner           # parameter playground — planned (P3+)
```

## Quick start

Rust toolchain is pinned in `rust-toolchain.toml`; any GPU that wgpu supports (Vulkan / Metal / DX12) works.

```sh
cargo run -p minimal
```

A gradient backdrop with a draggable glass panel opens. Dragging springs with a jelly feel — the silhouette stretches along the motion and the refraction lags behind (see [the optics report §7](docs/optics.en.md)). Drop an image onto the window (or pass `--image path/to.png`, or use the file picker) to refract your own picture; the egui panel tunes IOR, bevel, dispersion, and Fresnel live.

```sh
cargo run -p live-backdrop
```

The P2 acceptance demo: the glass refracts a scene the example **renders itself** (aurora + grid). Two panels start deliberately overlapping — press `M` to switch between Stack (later panel covers) and Merge (panels fuse into one continuous glass body).

## Roadmap

- [x] **P0** — strip jeantimex's glass shader into a DOM-free, pure-texture-input version
- [x] **P1** — minimal demo: SDF rounded panel, dragging, full refraction + dispersion + Fresnel (core acceptance)
- [x] **P2** — offscreen live-backdrop pipeline + multi-panel compositor (Stack/Merge)
- [x] **P3** — `vitreo-egui` binding crate: spring panels + shader-level jelly deformation (velocity squash-stretch, normal lag, press shrink — §7 of the optics report)
- [ ] **P4** — publish `vitreo` 0.1 + showcase

## Documentation

- [The Optics of Vitreo](docs/optics.en.md) / [Vitreo 的光学](docs/optics.zh.md) — the physics report: every formula, every constant, every figure derived from the source
- [docs/figures/generate.py](docs/figures/generate.py) — bilingual figure pipeline; re-runs the shader math in NumPy (parity oracle)
- [AGENTS.md](AGENTS.md) — working agreement for AI contributors (parity invariant, commit rules, license rules)

## License & acknowledgements

Vitreo is [MIT](LICENSE)-licensed. Built on the shoulders of these MIT projects — thank you:

- [jeantimex/glass-effect-webgpu](https://github.com/jeantimex/glass-effect-webgpu) — WGSL refraction shader lineage (thickness/bezel/refraction/blur/specular uniform pipeline), README credit
- [m2-md/liquid-glass-refraction-shader](https://github.com/m2-md/liquid-glass-refraction-shader) — origin of the IOR, dispersion, and Fresnel math; `optics.ts` / `sdf2d.ts` and its 84 tests ported to Rust unit tests
- [heonny/egui-glass](https://github.com/heonny/egui-glass) — egui-wgpu LiveBackdrop offscreen rendering skeleton reference
- [charlie-x/liquidImgui and forks](https://github.com/charlie-x/liquidImgui) — Windows DXGI Desktop Duplication desktop-composition reference
- [zaroutt/Niri-glass](https://github.com/zaroutt/Niri-glass) — compositor-level SDF refraction in GLSL (Wayland)
- [OverShifted/LiquidGlass](https://github.com/OverShifted/LiquidGlass) — C++/OpenGL SDF refraction reference
