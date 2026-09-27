#!/usr/bin/env python3
"""生成《The Optics of Vitreo》文档的全部插图。

图中所有曲线都逐公式复刻自 vitreo 源码（不是示意图）：
  - sd_rounded_box / glass_height / glass_normal / sdf_union  ← vitreo/src/sdf2d.rs
  - refract / refract_offset_px / fresnel_schlick ← vitreo/src/optics.rs
  - spectrum_weight / dispersed_ior               ← vitreo/src/glass.wgsl
Rust 侧数学改动时请同步本文件。

用法（仓库根目录）:
    .venv/bin/python docs/figures/generate.py

输出: docs/figures/*.svg(文档引用) 与 *.png(预览/验收用)。
"""

from __future__ import annotations

import math
from pathlib import Path

import matplotlib

matplotlib.use("Agg")

import numpy as np
from matplotlib import font_manager
from matplotlib.colors import PowerNorm
from matplotlib.lines import Line2D
from matplotlib.patches import Arc, FancyArrowPatch

import matplotlib.pyplot as plt

OUT = Path(__file__).parent

# ---------- 物理常数（与 vitreo/src/optics.rs 一致） ----------
IOR_AIR = 1.0
IOR_WATER = 1.333
IOR_ACRYLIC = 1.49
IOR_CROWN_GLASS = 1.52
IOR_SAPPHIRE = 1.77
IOR_DIAMOND = 2.417

# BK7 的两点 Cauchy 拟合（见 dispersion 一节），波长单位 µm。
BK7_ND, BK7_ABBE = 1.5168, 64.17
BK7_CAUCHY_B = ((BK7_ND - 1.0) / BK7_ABBE) / (1 / 0.4861**2 - 1 / 0.6563**2)
BK7_CAUCHY_A = BK7_ND - BK7_CAUCHY_B / 0.5876**2

# Paul Tol 柔和色（色盲友好），正文图统一用这套。
C_BLUE, C_CYAN, C_GREEN = "#4477AA", "#66CCEE", "#228833"
C_YELLOW, C_RED, C_PURPLE, C_GREY = "#CCBB44", "#EE6677", "#AA3377", "#BBBBBB"

# ---------- Vitreo 默认材质（与 vitreo/src/style.rs 一致） ----------
STYLE_BEVEL = 34.0
STYLE_THICKNESS = 6.0
STYLE_DEPTH = 90.0
STYLE_DISPERSION = 0.15


def setup_fonts() -> None:
    """注册系统中文字体，让插图里的中文标签正常显示。

    关键点：含 $...$ 的整条字符串都会走 mathtext 渲染，而 mathtext 字体
    不做 CJK 逐字形回退，所以必须把 mathtext 也指到同一颗全字形字体上，
    否则「中文 + 公式」混排会出现豆腐块。
    """
    for path in [
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/Supplemental/Songti.ttc",
    ]:
        if Path(path).exists():
            try:
                font_manager.fontManager.addfont(path)
            except Exception:
                pass  # 个别 .ttc 版本 matplotlib 读不了，靠下一个候选
    names = {f.name for f in font_manager.fontManager.ttflist}
    primary = next(
        (n for n in ["Arial Unicode MS", "Hiragino Sans GB", "Songti SC"] if n in names),
        "sans-serif",
    )
    plt.rcParams["font.family"] = [primary, "DejaVu Sans"]
    plt.rcParams["mathtext.fontset"] = "custom"
    plt.rcParams["mathtext.rm"] = primary
    plt.rcParams["mathtext.it"] = primary
    plt.rcParams["mathtext.bf"] = primary
    plt.rcParams["mathtext.default"] = "regular"
    plt.rcParams["axes.unicode_minus"] = False


# ---------- 与 sdf2d.rs 逐行对应的几何数学 ----------
def sd_rounded_box(px, py, bx, by, r):
    qx = np.abs(px) - bx + r
    qy = np.abs(py) - by + r
    inner = np.minimum(np.maximum(qx, qy), 0.0)
    outer = np.hypot(np.maximum(qx, 0.0), np.maximum(qy, 0.0))
    return inner + outer - r


def glass_height(d, w):
    x = np.clip(-d / max(w, 1e-3), 0.0, 1.0)
    t = 1.0 - x
    return np.sqrt(np.maximum(1.0 - t * t, 0.0))


def glass_normal(px, py, half_size, radius, bevel, thickness, eps=1.0):
    hx = glass_height(sd_rounded_box(px + eps, py, *half_size, radius), bevel) - glass_height(
        sd_rounded_box(px - eps, py, *half_size, radius), bevel
    )
    hy = glass_height(sd_rounded_box(px, py + eps, *half_size, radius), bevel) - glass_height(
        sd_rounded_box(px, py - eps, *half_size, radius), bevel
    )
    k = thickness / (2.0 * eps)
    x, y = -hx * k, -hy * k
    length = np.sqrt(x * x + y * y + 1.0)
    return x / length, y / length, 1.0 / length


# ---------- 与 optics.rs 逐行对应的光学数学 ----------
def refract(i, n, eta):
    ni = i[0] * n[0] + i[1] * n[1] + i[2] * n[2]
    k = 1.0 - eta * eta * (1.0 - ni * ni)
    if k < 0.0:
        return np.zeros(3)
    s = eta * ni + math.sqrt(k)
    return eta * np.asarray(i) - s * np.asarray(n)


def refract_offset_px(n, eta, depth_px):
    r = refract((0.0, 0.0, -1.0), n, eta)
    if not np.any(r):
        return np.zeros(2)
    rz = max(abs(r[2]), 1e-3)
    return np.array([r[0] * depth_px / rz, r[1] * depth_px / rz])


def schlick_f0(n1, n2):
    r = (n1 - n2) / (n1 + n2)
    return r * r


def fresnel_schlick(cos_theta, f0):
    c = np.clip(1.0 - cos_theta, 0.0, 1.0)
    return f0 + (1.0 - f0) * c**5


def fresnel_exact(theta_i, n1, n2):
    """电介质精确 Fresnel 反射率，返回 (Rs, Rp)。入射角弧度制，从 n1 一侧入射。"""
    sin_t = n1 / n2 * np.sin(theta_i)
    cos_t = np.sqrt(np.maximum(1.0 - sin_t**2, 0.0))
    cos_i = np.cos(theta_i)
    rs = ((n1 * cos_i - n2 * cos_t) / (n1 * cos_i + n2 * cos_t)) ** 2
    rp = ((n1 * cos_t - n2 * cos_i) / (n1 * cos_t + n2 * cos_i)) ** 2
    return rs, rp


def wavelength_rgb(nm):
    """可见光波长 → 近似 sRGB（Dan Bruton 算法），仅供绘图上色。"""
    nm = np.asarray(nm, dtype=float)
    r = np.zeros_like(nm)
    g = np.zeros_like(nm)
    b = np.zeros_like(nm)
    m1 = (nm >= 380) & (nm < 440)
    r[m1], b[m1] = -(nm[m1] - 440) / 60, 1.0
    m2 = (nm >= 440) & (nm < 490)
    g[m2], b[m2] = (nm[m2] - 440) / 50, 1.0
    m3 = (nm >= 490) & (nm < 510)
    g[m3], b[m3] = 1.0, -(nm[m3] - 510) / 20
    m4 = (nm >= 510) & (nm < 580)
    r[m4], g[m4] = (nm[m4] - 510) / 70, 1.0
    m5 = (nm >= 580) & (nm < 645)
    r[m5] = 1.0
    m6 = (nm >= 645) & (nm <= 700)
    r[m6] = 1.0
    dim = np.clip((440 - nm) / 60, 0, 1) * (nm < 440) + np.clip((nm - 645) / 80, 0, 1) * (nm > 645)
    gamma = 0.8
    out = np.stack([np.clip((1 - dim) * c, 0, 1) ** gamma for c in (r, g, b)], axis=-1)
    return np.clip(out * 0.85 + 0.15, 0, 1)  # 压暗提亮，压住白底


def save(fig, name):
    fig.savefig(OUT / f"{name}.svg", bbox_inches="tight")
    fig.savefig(OUT / f"{name}.png", dpi=180, bbox_inches="tight")
    plt.close(fig)
    print(f"  wrote {name}.svg / {name}.png")


# ======================================================================
# Fig. 1 — Snell 折射 + 全内反射
# ======================================================================
def fig_snell():
    fig, (ax_a, ax_b) = plt.subplots(1, 2, figsize=(10.4, 4.5))

    def draw_interface(ax, top_color, bottom_color):
        ax.axhspan(0, 2.0, color=top_color, zorder=0)
        ax.axhspan(-2.0, 0, color=bottom_color, zorder=0)
        ax.axhline(0, color="k", lw=1.4, zorder=3)
        ax.plot([0, 0], [-1.5, 1.5], ls="--", color="0.35", lw=1.0, zorder=2)

    def ray(ax, start, direction, color, lw=2.2, ls="-", label=None):
        end = start + direction
        ax.add_patch(
            FancyArrowPatch(
                start, end, arrowstyle="-|>", mutation_scale=14,
                color=color, lw=lw, linestyle=ls, zorder=4, shrinkA=0, shrinkB=0,
            )
        )
        if label:
            ax.annotate(label, (start + end) / 2, textcoords="offset points",
                        xytext=(6, 6), fontsize=9, color=color)

    # (a) 空气 → 玻璃：折向法线
    ax = ax_a
    draw_interface(ax, "#f7f9fc", "#dde8f5")
    theta1 = math.radians(50)
    theta2 = math.asin(math.sin(theta1) / IOR_CROWN_GLASS)
    ray(ax, np.array([-1.55 * math.sin(theta1), 1.55 * math.cos(theta1)]),
        np.array([1.55 * math.sin(theta1), -1.55 * math.cos(theta1)]), C_RED)
    ray(ax, np.zeros(2),
        np.array([1.55 * math.sin(theta2), -1.55 * math.cos(theta2)]), C_BLUE)
    ax.add_patch(Arc((0, 0), 1.1, 1.1, theta1=90, theta2=90 + math.degrees(theta1),
                     color=C_RED, lw=1.4))
    ax.add_patch(Arc((0, 0), 1.1, 1.1, theta1=270, theta2=270 + math.degrees(theta2),
                     color=C_BLUE, lw=1.4))
    ax.annotate(r"$\theta_1 = 50°$", (0.62 * math.cos(math.radians(115)), 0.62 * math.sin(math.radians(115))),
                ha="center", va="center", fontsize=10, color=C_RED)
    ax.annotate(rf"$\theta_2 = {math.degrees(theta2):.1f}°$",
                (0.68 * math.cos(math.radians(295)), 0.68 * math.sin(math.radians(295))),
                ha="center", va="center", fontsize=10, color=C_BLUE)
    # 介质标签用 Unicode 下标，避免「中文 + $公式」混排触发 mathtext 的 CJK 缺字。
    ax.text(-1.95, 1.78, "air 空气 · n₁ = 1.00", fontsize=10, ha="left")
    ax.text(-1.95, -1.88, "crown glass 冕牌玻璃 · n₂ = 1.52", fontsize=10, ha="left")
    ax.text(-0.09, 1.42, "normal 法线", fontsize=8.5, color="0.35", rotation=90,
            va="top", ha="right")
    ax.set_title("(a) entering a denser medium: bent toward the normal\n"
                 "进入光密介质：折向法线", fontsize=10)
    ax.text(0.5, -0.32, r"$n_1 \sin\theta_1 = n_2 \sin\theta_2$",
            transform=ax.transAxes, ha="center", fontsize=13)

    # (b) 玻璃 → 空气：超过临界角 → 全内反射
    ax = ax_b
    draw_interface(ax, "#f7f9fc", "#dde8f5")
    theta_c = math.degrees(math.asin(1.0 / IOR_CROWN_GLASS))
    theta1 = math.radians(45)  # > 临界角
    ray(ax, np.array([-1.55 * math.sin(theta1), -1.55 * math.cos(theta1)]),
        np.array([1.55 * math.sin(theta1), 1.55 * math.cos(theta1)]), C_RED)
    ray(ax, np.zeros(2),
        np.array([1.55 * math.sin(theta1), -1.55 * math.cos(theta1)]), C_PURPLE, ls="--")
    # 隐失波：沿界面短暂传播
    ax.add_patch(FancyArrowPatch((0.12, 0), (1.05, 0), arrowstyle="-|>", mutation_scale=12,
                                 color=C_GREY, lw=1.6, linestyle=":", zorder=4))
    ax.text(0.62, 0.1, "evanescent wave 隐失波", fontsize=8.5, color="0.45", ha="center")
    ax.add_patch(Arc((0, 0), 1.1, 1.1, theta1=270 - math.degrees(theta1), theta2=270,
                     color=C_RED, lw=1.4))
    ax.add_patch(Arc((0, 0), 1.1, 1.1, theta1=270, theta2=270 + math.degrees(theta1),
                     color=C_PURPLE, lw=1.4))
    # 纯公式（无中文）走 mathtext 安全；放左下角空白处，避开光线。
    ax.text(0.035, 0.05, rf"$\theta_1 = 45° > \theta_c = {theta_c:.1f}°$",
            transform=ax.transAxes, fontsize=10, color=C_RED)
    ax.annotate("reflected 反射光线", (1.0 * math.sin(theta1), -1.0 * math.cos(theta1)),
                fontsize=9, color=C_PURPLE, ha="left", xytext=(10, -12),
                textcoords="offset points")
    ax.text(-1.95, 1.78, "air 空气 · n = 1.00", fontsize=10, ha="left")
    ax.text(-1.95, -1.88, "crown glass 冕牌玻璃 · n = 1.52", fontsize=10, ha="left")
    ax.set_title("(b) beyond the critical angle: total internal reflection\n"
                 "超过临界角：全内反射（refract() 返回零向量）", fontsize=10)

    for ax in (ax_a, ax_b):
        ax.set_xlim(-2.05, 2.05)
        ax.set_ylim(-2.05, 2.05)
        ax.set_aspect("equal")
        ax.axis("off")

    fig.tight_layout()
    save(fig, "snell")


# ======================================================================
# Fig. 2 — 精确 Fresnel vs Schlick 近似（空气 → 冕牌玻璃）
# ======================================================================
def fig_fresnel():
    fig, (ax, ax_err) = plt.subplots(
        2, 1, figsize=(8.6, 5.8), sharex=True,
        gridspec_kw={"height_ratios": [2.3, 1.0], "hspace": 0.12},
    )
    theta = np.linspace(0, math.pi / 2, 901)
    rs, rp = fresnel_exact(theta, IOR_AIR, IOR_CROWN_GLASS)
    unpol = (rs + rp) / 2
    f0 = schlick_f0(IOR_AIR, IOR_CROWN_GLASS)
    schlick = fresnel_schlick(np.cos(theta), f0)
    deg = np.degrees(theta)

    theta_b = math.degrees(math.atan(IOR_CROWN_GLASS))

    ax.plot(deg, rs, color=C_BLUE, lw=2, label=r"$R_s$ · s 偏振 (exact 精确)")
    ax.plot(deg, rp, color=C_GREEN, lw=2, label=r"$R_p$ · p 偏振 (exact 精确)")
    ax.plot(deg, unpol, color=C_GREY, lw=1.6, ls="-", label="unpolarized mean 自然光平均")
    ax.plot(deg, schlick, color=C_RED, lw=2.2, ls="--",
            label="Schlick approximation Schlick 近似")
    ax.axvline(theta_b, color=C_PURPLE, lw=1.0, ls=":")
    ax.annotate(f"Brewster angle 布儒斯特角\n$\\theta_B = {theta_b:.1f}°$, $R_p \\to 0$",
                xy=(theta_b, 0.02), xytext=(theta_b + 6, 0.30), fontsize=9, color=C_PURPLE,
                arrowprops=dict(arrowstyle="->", color=C_PURPLE, lw=1.0))
    ax.annotate(f"$F_0 = \\left(\\frac{{n_1 - n_2}}{{n_1 + n_2}}\\right)^2 \\approx {f0 * 100:.2f}\\%$",
                xy=(0, f0), xytext=(26, 0.16), fontsize=9.5,
                arrowprops=dict(arrowstyle="->", lw=1.0, color="0.3"), color="0.15")
    ax.set_ylabel("Reflectance 反射率 $R$")
    ax.set_ylim(0, 1.02)
    ax.set_title("Fresnel reflectance, air → crown glass ($n = 1.52$)\n"
                 "Fresnel 反射率：空气 → 冕牌玻璃", fontsize=11)
    ax.legend(loc="upper left", fontsize=8.5, framealpha=0.95)
    ax.grid(alpha=0.25)

    err = (schlick - unpol) * 100
    ax_err.plot(deg, err, color=C_RED, lw=1.8)
    ax_err.fill_between(deg, err, 0, color=C_RED, alpha=0.15)
    ax_err.axvline(theta_b, color=C_PURPLE, lw=1.0, ls=":")
    i_max = np.argmax(np.abs(err))
    ax_err.annotate(f"max error 最大误差 ≈ {err[i_max]:+.2f} pp @ {deg[i_max]:.0f}°",
                    xy=(deg[i_max], err[i_max]), xytext=(deg[i_max] - 42, err[i_max] + 0.9),
                    fontsize=9, arrowprops=dict(arrowstyle="->", lw=1.0, color="0.3"))
    ax_err.set_xlabel("Angle of incidence 入射角 $\\theta_i$ (degrees 度)")
    ax_err.set_ylabel("Schlick error 误差\n(percentage points 百分点)")
    ax_err.grid(alpha=0.25)

    save(fig, "fresnel")


# ======================================================================
# Fig. 3 — 色散：BK7 的 Cauchy 曲线 + 物理/艺术化色散的位移对比
# ======================================================================
def fig_dispersion():
    fig, (ax_n, ax_off) = plt.subplots(1, 2, figsize=(11.2, 4.3))

    # (a) 正常色散 n(λ)
    lam = np.linspace(400, 700, 600)
    n = BK7_CAUCHY_A + BK7_CAUCHY_B / (lam / 1000) ** 2
    rgb = wavelength_rgb(lam)
    for i in range(len(lam) - 1):
        ax_n.plot(lam[i : i + 2], n[i : i + 2], color=rgb[i], lw=2.6, solid_capstyle="round")
    for nm, label, color in [(486.1, "F line 谱线 (486 nm, blue 蓝)", "#3b6fd4"),
                             (587.6, "d line 谱线 (588 nm, yellow 黄)", "#b3a11c"),
                             (656.3, "C line 谱线 (656 nm, red 红)", "#c33b3b")]:
        ni = BK7_CAUCHY_A + BK7_CAUCHY_B / (nm / 1000) ** 2
        ax_n.axvline(nm, color=color, ls=":", lw=1.1)
        ax_n.plot(nm, ni, "o", color=color, ms=4.5)
    ax_n.annotate("F 486", (486.1, BK7_CAUCHY_A + BK7_CAUCHY_B / 0.4861**2),
                  xytext=(430, 1.5235), fontsize=8.5, color="#3b6fd4",
                  arrowprops=dict(arrowstyle="->", color="#3b6fd4", lw=0.8))
    ax_n.annotate("d 588", (587.6, BK7_ND), xytext=(545, 1.5190), fontsize=8.5, color="#8a7d0e",
                  arrowprops=dict(arrowstyle="->", color="#8a7d0e", lw=0.8))
    ax_n.annotate("C 656", (656.3, BK7_CAUCHY_A + BK7_CAUCHY_B / 0.6563**2),
                  xytext=(620, 1.5155), fontsize=8.5, color="#c33b3b",
                  arrowprops=dict(arrowstyle="->", color="#c33b3b", lw=0.8))
    ax_n.set_xlabel("Wavelength 波长 $\\lambda$ (nm)")
    ax_n.set_ylabel("Refractive index 折射率 $n(\\lambda)$")
    ax_n.set_title("(a) normal dispersion of BK7 glass:\n"
                   "$n(\\lambda) = A + B/\\lambda^2$   BK7 玻璃的正常色散", fontsize=10)
    ax_n.text(0.04, 0.06,
              "Abbe number 阿贝数\n$V_d = \\frac{n_d - 1}{n_F - n_C} = 64.17$\n"
              "$n_F - n_C \\approx 0.0081$",
              transform=ax_n.transAxes, fontsize=9, va="bottom",
              bbox=dict(boxstyle="round,pad=0.4", fc="white", ec="0.7", alpha=0.9))
    ax_n.grid(alpha=0.25)

    # (b) 位移谱：物理 BK7 vs Vitreo 默认艺术化色散
    n_probe = np.array([0.6, 0.0, 0.8])  # bevel 上典型法线，与单测一致
    eta_phys = 1.0 / n
    off_phys = np.array([np.linalg.norm(refract_offset_px(n_probe, e, STYLE_DEPTH)) for e in eta_phys])
    t = (656.3 - lam) / (656.3 - 486.1)  # 红 0 → 蓝 1，与 shader 的 t 扫掠一致
    n_art = IOR_CROWN_GLASS + STYLE_DISPERSION * (t - 0.5)
    off_art = np.array([np.linalg.norm(refract_offset_px(n_probe, 1.0 / ni, STYLE_DEPTH)) for ni in n_art])

    for arr, alpha, lw in [(off_phys, 1.0, 2.2), (off_art, 0.45, 4.5)]:
        for i in range(len(lam) - 1):
            ax_off.plot(lam[i : i + 2], arr[i : i + 2], color=rgb[i], lw=lw, alpha=alpha,
                        solid_capstyle="round")
    span_phys = off_phys[-1] - off_phys[0]
    span_art = off_art[-1] - off_art[0]
    off_phys_f = float(np.interp(486.1, lam, off_phys))
    ax_off.annotate(f"BK7 physical 物理色散: red–blue shift 红蓝差 ≈ {abs(span_phys):.2f} px",
                    xy=(486.1, off_phys_f), xytext=(430, 19.4), fontsize=9,
                    arrowprops=dict(arrowstyle="->", lw=0.9, color="0.3"))
    ax_off.text(505, 24.25,
                f"Vitreo default 默认 dispersion 0.15\nred–blue shift 红蓝差 ≈ {abs(span_art):.1f} px",
                fontsize=9, ha="left")
    ax_off.set_xlabel("Wavelength 波长 $\\lambda$ (nm)")
    ax_off.set_ylabel("Displacement 位移 $|\\delta|$ (px)")
    ax_off.set_ylim(17.8, 26.2)
    ax_off.set_title("(b) screen-space displacement spectrum\n"
                     "bevel 法线 (0.6, 0, 0.8)，depth = 90 px：屏幕空间位移随波长", fontsize=10)
    ax_off.legend(handles=[
        Line2D([], [], color="0.35", lw=2.2, label="BK7 physical 物理色散 (F–C Δn ≈ 0.008)"),
        Line2D([], [], color="0.65", lw=4.5, alpha=0.45, label="Vitreo default 默认 (Δn = 0.15)"),
    ], loc="upper right", fontsize=8.5)
    ax_off.grid(alpha=0.25)

    fig.tight_layout()
    save(fig, "dispersion")


# ======================================================================
# Fig. 4 — bevel 剖面 + 屏幕空间位移场
# ======================================================================
def fig_bevel():
    fig, (ax_p, ax_f) = plt.subplots(1, 2, figsize=(11.4, 4.6))

    # (a) 剖面：height T·h(d)，法线箭头
    d = np.linspace(-70, 25, 960)
    h = glass_height(d, STYLE_BEVEL)
    z = STYLE_THICKNESS * h
    ax_p.fill_between(d, z, 0, color="#dde8f5", zorder=1)
    ax_p.plot(d, z, color=C_BLUE, lw=2.4, zorder=3)
    # 法线箭头（与 glass_normal 的公式一致）
    for di in [-6, -12, -18, -24, -30, -36, -46]:
        hi = glass_height(np.array([di]), STYLE_BEVEL)[0]
        zi = STYLE_THICKNESS * hi
        eps = 0.5
        slope = (glass_height(np.array([di + eps]), STYLE_BEVEL)[0]
                 - glass_height(np.array([di - eps]), STYLE_BEVEL)[0]) / (2 * eps)
        nx, nz = -STYLE_THICKNESS * slope, 1.0
        length = math.hypot(nx, nz)
        nx, nz = nx / length, nz / length
        ax_p.annotate(
            "", xy=(di + nx * 7, zi + nz * 7), xytext=(di, zi),
            arrowprops=dict(arrowstyle="-|>", color=C_RED, lw=1.4, mutation_scale=11))
        if di == -24:
            ax_p.text(di + nx * 7 + 1.0, zi + nz * 7 + 0.5, "$\\mathbf{n}$",
                      color=C_RED, fontsize=11)
    ax_p.annotate("", xy=(0, 8.6), xytext=(-STYLE_BEVEL, 8.6),
                  arrowprops=dict(arrowstyle="<->", color="0.25", lw=1.1))
    ax_p.text(-STYLE_BEVEL / 2, 9.2, "bevel width $w$ = 34 px 边缘弯曲区宽度",
              ha="center", fontsize=9, color="0.25")
    ax_p.annotate("", xy=(14.5, 0), xytext=(14.5, STYLE_THICKNESS),
                  arrowprops=dict(arrowstyle="<->", color="0.25", lw=1.1))
    ax_p.text(17, STYLE_THICKNESS / 2, "thickness $T$ = 6 px\n剖面高度", fontsize=9,
              color="0.25", va="center")
    ax_p.axvspan(-STYLE_BEVEL, 0, color=C_YELLOW, alpha=0.13, zorder=0)
    ax_p.text(-STYLE_BEVEL / 2, -2.6, "bevel 边缘弯曲区", ha="center", fontsize=8.5, color="#8a7d0e")
    ax_p.text(-52, -2.6, "plateau 平坦区", ha="center", fontsize=8.5, color="0.3")
    ax_p.axvline(0, color="0.5", ls=":", lw=1.0)
    ax_p.text(1.5, 5.4, "$d = 0$\nedge 边缘", fontsize=8.5, color="0.4")
    ax_p.set_xlabel("signed distance to panel edge 到面板边缘的有符号距离 $d$ (px)")
    ax_p.set_ylabel("surface height 表面高度 $T\\,h(d, w)$ (px)")
    ax_p.set_title("(a) quarter-circle bevel profile and normals\n"
                   "四分之一圆弧 bevel 剖面与表面法线", fontsize=10)
    ax_p.set_xlim(-70, 25)
    ax_p.set_ylim(-4, 12)
    ax_p.grid(alpha=0.25)

    # (b) 位移场热图：整块面板 |δ|
    half = (210.0, 130.0)
    radius = 64.0
    xs = np.arange(-half[0] - 20, half[0] + 20 + 1, 1.0)
    ys = np.arange(-half[1] - 20, half[1] + 20 + 1, 1.0)
    px, py = np.meshgrid(xs, ys)
    nx, ny, nz = glass_normal(px, py, half, radius, STYLE_BEVEL, STYLE_THICKNESS)
    # refract_offset_px 的数组展开：I = (0, 0, -1)，n·I = -nz
    eta = 1.0 / IOR_CROWN_GLASS
    k = 1.0 - eta * eta * (1.0 - nz * nz)
    k = np.clip(k, 1e-9, None)
    s = eta * (-nz) + np.sqrt(k)
    rx = eta * 0.0 - s * nx
    ry = eta * 0.0 - s * ny
    rz = eta * -1.0 - s * nz
    mag = np.hypot(rx, ry) * STYLE_DEPTH / np.maximum(np.abs(rz), 1e-3)

    im = ax_f.imshow(
        mag, extent=[xs[0], xs[-1], ys[-1], ys[0]], aspect="equal",
        cmap="viridis", origin="upper",
    )
    cbar = fig.colorbar(im, ax=ax_f, fraction=0.045, pad=0.02)
    cbar.set_label("Displacement 位移 $|\\delta|$ (px)", fontsize=9)
    # 面板轮廓 d=0
    ax_f.contour(px, py, sd_rounded_box(px, py, *half, radius), levels=[0],
                 colors="white", linewidths=1.4)
    ax_f.set_xlabel("x (px)")
    ax_f.set_ylabel("y (px, screen down 屏幕向下)")
    ax_f.set_title("(b) screen-space displacement field $|\\delta|$\n"
                   "420×260 panel, $r$=64: 屏幕空间位移场（折射透镜效果集中在边缘）", fontsize=10)

    fig.tight_layout()
    save(fig, "bevel")


# ======================================================================
# Fig. 5 — 多面板合成：Stack vs Merge（并集 SDF 法线）
# ======================================================================
def fig_merge():
    # 与 vitreo/src/sdf2d.rs merge_tests 相同的几何：
    # A 中心 (0,0)，B 中心 (260,0)，半尺寸 (210,130)，r = 40，B 在上层。
    half = (210.0, 130.0)
    radius = 40.0
    offset_b = 260.0
    eps = 1.0  # NORMAL_EPS，与 WGSL/CPU oracle 一致

    def d_a(x, y):
        return sd_rounded_box(x, y, *half, radius)

    def d_b(x, y):
        return sd_rounded_box(x - offset_b, y, *half, radius)

    def d_union(x, y):
        return np.minimum(d_a(x, y), d_b(x, y))  # sdf_union：CSG 并集

    def tilt(dfunc, px, py):
        # glass_normal / union_normal 的平面内倾角 ‖n_xy‖（式 2.3 的差分公式，
        # 与 Rust 侧一样做单位化）
        hx = glass_height(dfunc(px + eps, py), STYLE_BEVEL) - glass_height(
            dfunc(px - eps, py), STYLE_BEVEL
        )
        hy = glass_height(dfunc(px, py + eps), STYLE_BEVEL) - glass_height(
            dfunc(px, py - eps), STYLE_BEVEL
        )
        k = STYLE_THICKNESS / (2.0 * eps)
        nx, ny = -hx * k, -hy * k
        length = np.sqrt(nx * nx + ny * ny + 1.0)
        return np.hypot(nx, ny) / length

    xs = np.arange(-half[0] - 30, offset_b + half[0] + 31, 1.0)
    ys = np.arange(-half[1] - 30, half[1] + 31, 1.0)
    px, py = np.meshgrid(xs, ys)

    # (a) Stack：着色用"最上层的覆盖面板"自己的 SDF（B 覆盖处用 B，否则 A）。
    inside_b = d_b(px, py) < 0
    t_stack = np.where(inside_b, tilt(d_b, px, py), tilt(d_a, px, py))
    # (b) Merge：法线来自并集 SDF —— 倒角只留在融合后的外轮廓上。
    t_merge = tilt(d_union, px, py)

    outside = d_union(px, py) >= 0
    t_stack = np.ma.masked_where(outside, t_stack)
    t_merge = np.ma.masked_where(outside, t_merge)

    fig, axes = plt.subplots(1, 2, figsize=(12.6, 4.9))
    vmax = float(max(t_stack.max(), t_merge.max()))

    for ax, t, title in [
        (axes[0], t_stack,
         "(a) Stack: top panel shades with its own SDF\n"
         "Stack：上层用自身 SDF 着色，倒角环穿过重叠区内部"),
        (axes[1], t_merge,
         "(b) Merge: normals from the union SDF (fused)\n"
         "Merge：法线来自并集 SDF，融合成一块连续玻璃"),
    ]:
        im = ax.imshow(
            t, extent=[xs[0], xs[-1], ys[-1], ys[0]], aspect="equal",
            cmap="viridis", origin="upper",
            # 1px 边界 AA 尖峰 (~0.7) 会压扁 bevel 环带 (0.05–0.45) 的对比度，
            # 伽马 <1 拉伸低值区，让 (a) 的内部接缝与 (b) 的平坦重叠区一眼可辨。
            norm=PowerNorm(0.5, vmin=0.0, vmax=vmax),
        )
        ax.contour(px, py, d_a(px, py), levels=[0], colors="white",
                   linewidths=1.0, linestyles="--")
        ax.contour(px, py, d_b(px, py), levels=[0], colors="white",
                   linewidths=1.0, linestyles="--")
        ax.contour(px, py, d_union(px, py), levels=[0], colors="white", linewidths=1.8)
        ax.set_xlabel("x (px)")
        ax.set_ylabel("y (px, screen down 屏幕向下)")
        ax.set_title(title, fontsize=10)
        cbar = fig.colorbar(im, ax=ax, fraction=0.045, pad=0.02)
        cbar.set_label("normal tilt 法线倾角 $\\|\\mathbf{n}_{xy}\\|$", fontsize=9)

    axes[0].text(-105, 0, "A", color="white", fontsize=13, ha="center", va="center",
                 fontweight="bold")
    axes[0].text(offset_b + 105, 0, "B", color="white", fontsize=13, ha="center",
                 va="center", fontweight="bold")
    axes[0].annotate("interior seam 内部接缝",
                     (offset_b - half[0] + 10, -96), xytext=(offset_b - 30, -142),
                     color="white", fontsize=9, ha="center",
                     arrowprops=dict(arrowstyle="->", color="white", lw=1.0))
    axes[1].text(offset_b + 105, 0, "fused 融合体", color="white", fontsize=10,
                 ha="center", va="center")
    axes[1].annotate("seam eliminated 接缝消失",
                     (offset_b - half[0] + 10, -96), xytext=(offset_b - 30, -142),
                     color="white", fontsize=9, ha="center",
                     arrowprops=dict(arrowstyle="->", color="white", lw=1.0))

    fig.tight_layout()
    save(fig, "merge")


def main():
    setup_fonts()
    print("generating figures:")
    fig_snell()
    fig_fresnel()
    fig_dispersion()
    fig_bevel()
    fig_merge()

    # ---- 供文档引用的数值（打印出来核对） ----
    print("\nworked examples for the doc:")
    theta1 = math.radians(50)
    theta2 = math.asin(math.sin(theta1) / IOR_CROWN_GLASS)
    print(f"  Snell: 50 deg air->glass => {math.degrees(theta2):.2f} deg")
    print(f"  critical angle glass->air: {math.degrees(math.asin(1 / IOR_CROWN_GLASS)):.2f} deg")
    print(f"  Brewster angle: {math.degrees(math.atan(IOR_CROWN_GLASS)):.2f} deg")
    print(f"  F0 air/glass: {schlick_f0(1.0, IOR_CROWN_GLASS):.4f}")
    print(f"  BK7 Cauchy A={BK7_CAUCHY_A:.6f} B={BK7_CAUCHY_B:.6f} (um^2)")
    n_probe = np.array([0.6, 0.0, 0.8])
    for label, ior in [("water 水", IOR_WATER), ("crown glass 冕牌玻璃", IOR_CROWN_GLASS),
                       ("diamond 钻石", IOR_DIAMOND)]:
        off = np.linalg.norm(refract_offset_px(n_probe, 1.0 / ior, STYLE_DEPTH))
        print(f"  offset |delta| {label}: {off:.2f} px (depth 90)")
    lam = np.linspace(400, 700, 600)
    n_phys = BK7_CAUCHY_A + BK7_CAUCHY_B / (lam / 1000) ** 2
    t = (656.3 - lam) / (656.3 - 486.1)
    n_art = IOR_CROWN_GLASS + STYLE_DISPERSION * (t - 0.5)
    span = lambda ns: abs(
        np.linalg.norm(refract_offset_px(n_probe, 1.0 / ns[0], STYLE_DEPTH))
        - np.linalg.norm(refract_offset_px(n_probe, 1.0 / ns[-1], STYLE_DEPTH)))
    print(f"  red<->blue displacement span 红蓝位移差 (depth 90): "
          f"BK7 {span(n_phys):.2f} px, Vitreo 0.15 {span(n_art):.1f} px")

    # merge 算例（与 sdf2d.rs::merge_tests 同几何）：并集法线的 x 分量。
    def merge_probe_nx(x):
        y = 0.0

        def d(X):
            return min(
                float(sd_rounded_box(X, y, 210, 130, 40)),
                float(sd_rounded_box(X - 260, y, 210, 130, 40)),
            )

        hx = (glass_height(np.array([d(x + 1)]), STYLE_BEVEL)[0]
              - glass_height(np.array([d(x - 1)]), STYLE_BEVEL)[0])
        return float(-hx * STYLE_THICKNESS / 2.0)

    print("  merge union-normal nx (A + B at +260px, half 210x130, r 40):")
    print(f"    x =  60 (10px inside B's left edge, deep in A): {merge_probe_nx(60.0):+.4f}  (seam absorbed 接缝被吸收)")
    print(f"    x = 130 (overlap interior 重叠区内部):          {merge_probe_nx(130.0):+.4f}")
    print(f"    x = 460 (fused outer bevel 融合外轮廓 bevel):   {merge_probe_nx(460.0):+.4f}")


if __name__ == "__main__":
    main()
