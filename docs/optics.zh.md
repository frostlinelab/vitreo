# Vitreo 的光学

**屏幕空间玻璃渲染的物理模型**

*Vitreo 技术报告 1 · 2026-09-26 · 英文原文（主文档）：[optics.en.md](optics.en.md)*

---

## 摘要

Vitreo 用单次 wgpu 片元着色，把矩形玻璃面板叠加在任意二维背景之上。本文推导着色器求值的每一条公式——圆角盒有符号距离场、bevel 高度剖面及其表面法线、化为屏幕空间位移的 Snell 折射、由阿贝数驱动的色散、以及 Fresnel 反射率的 Schlick 近似——并对每个物理常数给出文献依据。文中所有图像曲线均由 `docs/figures/generate.py` 逐公式复刻 Rust/WGSL 源码计算得到，没有任何手绘示意。文末给出物理量到公开 `GlassStyle` 参数的映射，以及数值算例。

**关键词：** 折射，Snell 定律，有符号距离场，色散，阿贝数，Fresnel 方程，Schlick 近似，WGSL，wgpu

---

## 1. 引言

### 1.1 玻璃是介质，不是贴图

一块真实的玻璃不携带任何属于自己的图像。你*透过*它看到的和*在它表面上*看到的，完全由光与背后背景、光与玻璃两个表面的相互作用决定。这就是 Vitreo 的设计公理：

> **光学的部分跨平台共享，身份的部分每平台本地化。折射写一次，铬（chrome）各写各的。**

因此玻璃面板只需要一个必要输入——作为普通 RGBA 纹理的背景（静态图、应用自身场景的离屏渲染、或视频帧）——就能产生三种磨砂"模糊"无法复现的光学现象：

1. **折射**——透过面板看到的背景发生位移，且位移沿面板边缘变化；
2. **色散**——位移随波长不同，边缘出现光谱色镶边；
3. **Fresnel 反射**——面板边框在掠射角下捕捉光线、发出辉光。

### 1.2 管线总览

对面板内的每个像素，片元着色器（`vitreo/src/glass.wgsl`）依次求值：

```
sdRoundedBox SDF → bevel 高度场 → 表面法线
  → refract() → 屏幕空间位移（逐波长）
  → Schlick–Fresnel 边缘辉光 + 镜面高光
```

同一套数学在 CPU 侧存在两份——`vitreo/src/sdf2d.rs` 与 `vitreo/src/optics.rs`——作为对拍基准（parity oracle）：31 个单元测试钉死了数值行为，而 Rust 与 WGSL 一旦漂移，所有面板的形状都会悄悄改变。

### 1.3 记号约定

| 符号 | 含义 | 出处 |
|---|---|---|
| $n$ | 介质折射率（IOR） | §3 |
| $n_1, n_2$ | 入射侧 / 透射侧介质的折射率 | §3.2 |
| $\eta$（eta） | 送入 WGSL `refract` 的比值 $n_1 / n_2$ | §3.3 |
| $d$ | 到面板边缘的有符号距离（内部为负） | §2 |
| $w$ | bevel 宽度（像素） | §2.2 |
| $T$ | bevel 剖面高度（thickness，像素） | §2.2 |
| $D$ | 光程（depth，像素） | §3.4 |
| $V_d$ | 阿贝数 | §4 |
| $F$ | Fresnel 反射率 | §5 |

坐标均为物理像素，原点在视口左上角、$y$ 向下——与 wgpu 帧缓冲、winit 光标一致。

---

## 2. 几何：从圆角盒到表面法线

### 2.1 圆角盒 SDF

**有符号距离场**（signed distance field，SDF）是这样一种函数：对任意点 $p$ 返回到最近表面的距离——内部为负、外部为正、轮廓上恰为零。SDF 是程序化几何的通用语言，因为距离可以复合运算，抗锯齿也顺手可得。Vitreo 采用 Inigo Quilez 的二维圆角盒精确公式，在 `sdf2d.rs::sd_rounded_box` 与 `glass.wgsl::sd_rounded_box` 中逐字实现：

$$
\begin{aligned}
q &= |p| - b + r \\
d(p) &= \min(\max(q_x, q_y), 0) + \lVert \max(q, 0) \rVert - r
\end{aligned}
\tag{2.1}
$$

其中 $b$ 是盒子的**半尺寸**，$r$ 是圆角半径。两项把平面分成内部（$\max(q_x,q_y)<0$：轴向距离）、边缘与角附近的外部（欧氏距离）和远外部（欧氏距离）。三个单元测试钉死行为：中心处 $d = -b_y$，边缘上 $d = 0$，角点处 $d = (\sqrt2 - 1)\,r$——角点到四分之一圆弧恰距离 $r$。

### 2.2 bevel 高度剖面

一块完全平坦的板只在表面倾斜处弯折光线。"Liquid Glass"的观感来自 **bevel**：沿轮廓的一条宽度为 $w$ 的窄带，表面在其中从边缘向内部隆起，建模为高度 $T$（剖面*厚度*）的四分之一圆。`sdf2d.rs::glass_height` 把有符号距离映射为归一化高度：

$$
h(d, w) = \sqrt{1 - t^2}, \qquad t = 1 - \operatorname{clamp}\!\left(\frac{-d}{w}, 0, 1\right)
\tag{2.2}
$$

从里往外读：$-d/w$ 在边缘处为 0、向内 $w$ 像素处为 1；$t$ 反向（边缘为 1 → 内部为 0）；$1 - t^2$ 扫过四分之一圆；平方根补全它。于是边缘处 $h = 0$，平坦**台地**（plateau，$d \le -w$）上 $h = 1$，其间 $h$ 沿 $\sqrt{1-t^2}$ 走——恰好是从极点到赤道的单位圆右上四分之一。面板外（$d > 0$）clamp 同样给出 $h = 0$，这一点很重要，因为法线场（§2.3）会在轮廓外一两像素处求值以做抗锯齿。

物理表面高度即 $z(p) = T \cdot h(d(p), w)$——图 4(a) 的高度场。

> **为什么是四分之一圆？** 圆弧与台地相切（斜率 0——看不到接缝），与边缘正交（斜率 ∞——在边框处弯折最强，正是 Apple 风格玻璃透镜感最强的地方）。线性倒角两端都有斜率突变；四分之一圆在台地侧斜率连续、在边缘侧斜率无界。于是单个参数 $w$ 就控制了性格：$w$ 小读作硬质玻璃板，$w$ 大读作柔软水滴。

### 2.3 有限差分求表面法线

着色器需要每个像素处的单位法线 $\mathbf{n}$。Rust 基准与 WGSL 都不对复合场求解析导数，而是用 $\varepsilon = 1$ 像素步长的高度场中心差分（`sdf2d.rs::glass_normal`）：

$$
\begin{aligned}
h_x &= h\big(d(p + \varepsilon\hat{x})\big) - h\big(d(p - \varepsilon\hat{x})\big), \quad
h_y = h\big(d(p + \varepsilon\hat{y})\big) - h\big(d(p - \varepsilon\hat{y})\big)\\[4pt]
\mathbf{n} &= \frac{\left(-\dfrac{T}{2\varepsilon}h_x,\; -\dfrac{T}{2\varepsilon}h_y,\; 1\right)}{\left\lVert\cdot\right\rVert}
\end{aligned}
\tag{2.3}
$$

因子 $T/(2\varepsilon)$ 把有限差分斜率换算成真实像素高度——*thickness* 参数正是在此进入。台地上 $h_x = h_y = 0$、$\mathbf{n} = (0,0,1)$：不弯折，背景原样采样。bevel 上平面内分量增大，法线向外倾。单元测试断言：处处单位长度（含角点与外部）、关于坐标轴对称、台地上平坦、倾角随 $T$ 单调增大。

> **为什么用有限差分而不用解析梯度？** $\varepsilon = 1$ 像素与着色器的像素网格对齐，只需 4 次 SDF 求值，且在 CPU 基准上可精确复现。代价——法线场在 1 像素尺度上的轻微阶梯——在位移作用于平滑采样的背景之后不可见。

---

## 3. 折射：化为像素偏移的 Snell 定律

### 3.1 折射率

介质的**折射率**（index of refraction，IOR）$n$ 是真空中光速与该介质中相速度之比，$n = c / v$。它衡量光学密度：光进入更高 $n$ 的材料时变慢，波前发生偏转。$n$ 是真实的材料常数，不是风格化旋钮——`optics.rs` 内置物理表：

| 介质 | $n$ | 备注 |
|---|---|---|
| 空气 | 1.000 | 此处约定真空 ≈ 1 |
| 水 | 1.333 | |
| 亚克力（PMMA） | 1.490 | 真实 UI 硬件所用的"玻璃" |
| 冕牌玻璃 | **1.520** | Vitreo 默认；经典光学冕牌 |
| 蓝宝石 | 1.770 | 手表表镜 |
| 钻石 | 2.417 | 极限弯折 + 强色散 |

### 3.2 Snell 定律

光跨越 $n_1 \to n_2$ 的界面时，光线方向按 **Snell 定律**偏转（图 1a）：

$$
n_1 \sin\theta_1 = n_2 \sin\theta_2
\tag{3.1}
$$

进入光密介质（$n_2 > n_1$）时，光线*折向*法线（$\theta_2 < \theta_1$）：从空气以 50° 入射冕牌玻璃，透射角为 $\arcsin(\sin 50° / 1.52) = 30.26°$。

反方向则存在**临界角**，超过它就不存在透射：

$$
\theta_c = \arcsin\frac{n_1}{n_2} \quad (n_1 > n_2)
\tag{3.2}
$$

冕牌玻璃 → 空气的临界角为 $41.14°$。超过临界角，波无法在第二种介质中传播，**全内反射**（total internal reflection，TIR）把全部能量折回（图 1b）；场在技术上以贴着界面的**隐失波**（evanescent wave）形式延续。光纤与钻石的火彩正是这个效应。

### 3.3 WGSL `refract` 的约定

着色器使用的 GPU 内建函数——`optics.rs::refract` 逐行为一致地镜像了它——是：

$$
\operatorname{refract}(\mathbf{I}, \mathbf{N}, \eta), \qquad \eta = \frac{n_1}{n_2}
\tag{3.3}
$$

其中 $\mathbf{I}$ 是*指向*表面的单位入射方向，$\mathbf{N}$ 是*指向入射侧*的单位法线，且

$$
\mathbf{R} = \eta \mathbf{I} - \left(\eta\,(\mathbf{N}\!\cdot\!\mathbf{I}) + \sqrt{1 - \eta^2 (1 - (\mathbf{N}\!\cdot\!\mathbf{I})^2)}\right)\mathbf{N}
\tag{3.4}
$$

平方根项是 Snell 定律的判别式；当它变负（$\eta^2(1 - \cos^2\theta_i) > 1$）时无实解——TIR——内建函数返回零向量。单元测试钉死全部三种状态：正入射不弯折、斜入射满足 $\sin\theta_2 = \eta \sin\theta_1$、超过临界角返回零向量。

### 3.4 从光线到像素：屏幕空间位移

真实相机隔着玻璃斜视；UI 相机**正交地透过屏幕**看，$\mathbf{I} = (0, 0, -1)$，表面法线 $\mathbf{n} = (n_x, n_y, n_z)$ 几乎处处接近 $(0,0,1)$。应用式 (3.4)，让折射光线沿视线轴传播 $D$ 像素的光程，光线每单位深度的横向漂移为 $R_{xy} / |R_z|$，于是被采样背景像素的位移为（`optics.rs::refract_offset_px`、`glass.wgsl::refract_offset`）：

$$
\boldsymbol{\delta} = \frac{D}{|R_z|}\,\mathbf{R}_{xy} \quad\text{其中 } \mathbf{R} = \operatorname{refract}\big(\mathbf{I}, \mathbf{n},\, n_1/n_2\big)
\tag{3.5}
$$

分母 $|R_z|$ 把沿（基本轴向的）光线的路径长度换算成沿视线轴的深度。空气→玻璃折射（$\eta < 1$，不可能 TIR）下 $|R_z|$ 有界于约 $\eta$ 到 1 之间，除法总是良态的；代码里的 $1\,\mathrm{e}{-3}$ 下限是针对假想 $\eta > 1$ 配置的防御性保护，并非工作路径的必需。

位移随 $D$ **线性**增长（测试钉死），随 IOR 差**单调**增长。取典型 bevel 法线 $(0.6, 0, 0.8)$、$D = 90$ 像素：

| 材料 | $n$ | $\lVert\boldsymbol{\delta}\rVert$ |
|---|---|---|
| 水 | 1.333 | 16.06 px |
| 冕牌玻璃 | 1.520 | 21.81 px |
| 钻石 | 2.417 | 37.27 px |

> **`thickness` 与 `depth`：两个不同的物理量。** $T$（thickness，§2.2）是 bevel 横截面的*高度*——它塑造法线，即表面*在哪里*、*以多陡的角度*倾斜。$D$（depth，§3.4）是*光程*——它缩放倾斜表面对背景的推移量。薄而深的板拖出宽宽的位移带；厚而浅的板倾角很大却推移很少。两者都重要，UI 把它们分成两个参数正是为此。

### 3.5 眼睛为什么读出"这是玻璃"

式 (3.5) 解释了 Liquid Glass 的整体观感。台地上 $\boldsymbol\delta = 0$——背景无畸变地透过，"清澈"。越过 bevel，法线从竖直扫向掠射，$\lVert\delta\rVert$ 从 0 涨到几十像素再回落：边缘像一圈柱面透镜，恰在物理 bevel 该有的位置压缩、拉伸背景（图 4b）。人类视觉对透明固体*扭曲*边缘的方式极其敏感，读出的正是这种扭曲——而不是任何高光——"这是玻璃"。

---

## 4. 色散：每个波长一个 eta

### 4.1 物理色散与阿贝数

电介质的折射率随波长变化，因为光以波的频率极化材料中的束缚电子；越接近吸收共振，响应越强。在可见窗内——远离共振——$n(\lambda)$ 随 $\lambda$ 单调下降（**正常色散**），在有限区间上可用 Cauchy 两项式很好地近似：

$$
n(\lambda) = A + \frac{B}{\lambda^2}
\tag{4.1}
$$

对 **BK7**——光学最常见的硼硅冕牌玻璃——用 $d$ 谱线（587.6 nm，$n_d = 1.5168$）与 F/C 间距拟合得 $A = 1.504590$、$B = 0.004216\ \mu\text{m}^2$（图 3a）。玻璃行业用**阿贝数**（Abbe number）编目色散强度：

$$
V_d = \frac{n_d - 1}{n_F - n_C}
\tag{4.2}
$$

其中 $F = 486.1$ nm（蓝），$C = 656.3$ nm（红）。$V_d$ *高*意味着色散*低*。BK7 的 $V_d = 64.17$，故 $n_F - n_C = (1.5168-1)/64.17 = 0.00805$——不到 IOR 的百分之一。`optics.rs::abbe_spread` 实现的正是这个逆运算。

### 4.2 着色器中的逐通道折射

着色器在 F→C 区间循环 `SPECTRAL_SAMPLES = 3` 个采样点，$\eta_i = 1 / (n_d + \Delta \cdot (t_i - 0.5))$，$t_i \in \{0, \tfrac12, 1\}$——注意蓝端取*更高*的折射率、弯折更多（测试：蓝比红位移大）。每个采样点的位移取样按粗粒度三峰**光谱权重**

$$
w_k(t) = \exp\!\big(-16\,(t - \mu_k)^2\big), \quad \mu \in \{0.15, 0.50, 0.85\}
\tag{4.3}
$$

加权，结果逐通道除以权重和。等权进等权出：中性背景保持中性，只有三个采样点落在不同背景颜色上的像素才显出镶边。

### 4.3 诚实的差距：物理色散 vs 艺术化色散

严格按式 (4.2) 几乎不可见：经 bevel 法线、$D = 90$ 像素，BK7 的红蓝位移差只有 **0.48 像素**（图 3b 细曲线）——真实玻璃的物理诚实，而真实 UI 玻璃*就是*这么含蓄。Vitreo 默认 `dispersion = 0.15`——约为 BK7 展开的 19 倍——承袭参考实现的艺术化取值，把红蓝差推到 **7.0 像素**，让光谱镶边在 UI 尺度上可辨（图 3b 粗曲线）。需要物理正确值时用 `GlassStyle::bk7_dispersion()` 构造；`dispersion = 0` 会短路成单次 `refract` 求值。

> **三个采样点就够了。** 背景是被*空间地*位移，不是被*光谱地*重新着色；三 tap 加权平均就能复现视觉总体效果（内侧青色镶边、外侧红色镶边），不需要真实光谱积分的成本与噪声。这与游戏引擎做的取舍相同，而且它对"自己是近似"这件事很诚实。

---

## 5. Fresnel：玻璃边缘为什么会发光

### 5.1 精确 Fresnel 方程

在每个界面处，光分裂为反射与透射两部分。Augustin-Jean Fresnel 的方程给出电介质界面两种偏振态的精确反射率：

$$
R_s = \left(\frac{n_1\cos\theta_i - n_2\cos\theta_t}{n_1\cos\theta_i + n_2\cos\theta_t}\right)^{2}, \qquad
R_p = \left(\frac{n_1\cos\theta_t - n_2\cos\theta_i}{n_1\cos\theta_t + n_2\cos\theta_i}\right)^{2}
\tag{5.1}
$$

$\theta_t$ 由 Snell 定律给出。三个区间值得注意（图 2）：

- **正入射：** $R_s = R_p = \left(\frac{n_1-n_2}{n_1+n_2}\right)^2$。空气→冕牌玻璃：$((1-1.52)/2.52)^2 = 0.0426$——每个表面反射约 4%，即日常窗户玻璃的观感。
- **布儒斯特角** $\theta_B = \arctan(n_2/n_1) = 56.66°$：$R_p \to 0$——p 偏振光完全透射（偏光太阳镜正利用此效应）。
- **掠射** $\theta \to 90°$：两种偏振都 → 1。任何电介质在掠射角下都成为镜子——玻璃边框捕光发光靠的是这个，而不是染色。

### 5.2 Schlick 近似

逐像素求值式 (5.1) 需要两条 atan/sqrt 链加繁琐的分母保护。Christophe Schlick（1994）证明整个函数族可以坍缩为

$$
F(\cos\theta) \approx F_0 + (1 - F_0)\,(1 - \cos\theta)^5, \qquad F_0 = \left(\frac{n_1 - n_2}{n_1 + n_2}\right)^2
\tag{5.2}
$$

$F_0$ 是正入射反射率——唯一的材料特定量。该近似在 $\theta = 0°$ 与 $\theta = 90°$ 处精确，对 $n = 1.52$ 与精确自然光平均的最大偏差为 **+3.45 个百分点（85° 附近）**（图 2 下子图）——在 48 次幂的镜面瓣之下，这个误差人眼无法察觉。Vitreo 在 `optics.rs` 与 `glass.wgsl` 中逐字实现同一 `fresnel_schlick`，`fresnel_f0` 由 CPU 逐面板预计算（`panel.rs`），着色器无需重复计算。

### 5.3 合成中的边缘辉光

着色器按 Fresnel 因子把折射背景混向"天光"项，在 y 向下的屏幕坐标系里光自上方来：

$$
\text{sky} = \text{tint} \cdot (0.55 - 0.45\,n_y), \qquad
\text{color} = \operatorname{mix}(\text{refracted},\ \text{sky},\ F(\mathbf{n}\cdot\mathbf{v}))
\tag{5.3}
$$

平坦内部 → $F \approx F_0 \approx 4\%$：面板基本是一扇窗。边缘处 $\mathbf{n}$ 转向掠射 → $F \to 1$：背景让位于辉光。让你夜里窗玻璃微微照人的那个 $F_0 = 4.26\%$，正是 Liquid Glass 面板发光边框的来源——物理模型免费赚到了美学。

其上再叠加**镜面高光**：对主光方向 $(-0.35, -0.72, 0.60)$（y 翻转到屏幕坐标）取 Blinn-Phong 风格的 $\big(\mathbf{R}\cdot\mathbf{L}\big)^{48}$，按 `specular = 0.85` 缩放。物理上这是面板第二表面捕捉的"室内光"；艺术上是那条卖曲线的细亮痕。

---

## 6. 合成 pass：把面板装起来

逐像素，`fs_main` 自底向上遍历（至多 8 个）面板，最上层最后处理：

1. **覆盖率。** $c = 1 - \operatorname{smoothstep}(-1, 1, d)$ 把 SDF 变成 1 像素抗锯齿轮廓；面板外 $c = 0$。
2. **内部（$c > 0$）：** 按 §3–§5 着色，按 $c$ 混入运行中的颜色。
3. **外部且面板投影时：** SDF 软阴影——有符号距离按阴影向量偏移后过一个宽度为阴影模糊的 `smoothstep`——无需额外渲染目标即可压暗背景。
4. **背景模糊**（磨砂）是单次三线性 `textureSampleLevel`，$\mathrm{LOD} = \log_2(\text{模糊像素})$：mipmap 链就是预计算的 Gaussian 金字塔。廉价、稳定，也是采样器带线性 mipmap 的原因。
5. **染色**——可选颜色混合，兼作 §5.3 的辉光色。`tint_opacity = 0` 保持玻璃纯光学。

一切在线性光下进行；sRGB 编码由目标格式（`Bgra8UnormSrgb`）负责，因此上述物理常数无需修改即可使用。

---

## 7. 从物理到 API：`GlassStyle`

| 参数 | 物理量 | 默认值 | § |
|---|---|---|---|
| `ior` | 折射率 $n$ | 1.52（冕牌玻璃） | 3.1 |
| `bevel` | bevel 宽度 $w$（像素） | 34 | 2.2 |
| `thickness` | bevel 剖面高度 $T$（像素） | 6 | 2.2 |
| `depth` | 光程 $D$（像素） | 90 | 3.4 |
| `dispersion` | F–C 折射率差 $\Delta n$（0 = 关；`GlassStyle::bk7_dispersion()` = 0.008） | 0.15（艺术化） | 4 |
| `blur` | 背景磨砂半径（像素，mipmap LOD） | 0 | 6 |
| `specular` | 主光强度 | 0.85 | 5.3 |
| `tint` / `tint_opacity` | 染色与辉光色 | 白 / 0 | 6 |
| `shadow.*` | SDF 软阴影 | 关 | 6 |

## 8. 验证

- **31 个 CPU 单元测试**（`sdf2d.rs`、`optics.rs`、`panel.rs`）钉死：斜入射的 Snell 定律、超过临界角的零向量、随 $D$ 的线性增长、随 IOR 与 thickness 的单调增长、蓝比红弯折多、空气→冕牌玻璃的 $F_0 = 0.0426$、BK7 的 $\Delta n = 0.00805$、Fresnel 端点、uniform 布局不变量。
- **图像对拍。** 本文所有图像都由 Rust/WGSL 公式的逐行 NumPy 转写生成（`docs/figures/generate.py`）；光学数学任何改动后请重跑 `.venv/bin/python docs/figures/generate.py` 并比对。
- **数值算例**（每次生成图像时重新计算）：50° → 30.26°（空气→玻璃）；$\theta_c = 41.14°$、$\theta_B = 56.66°$；$F_0 = 4.26\%$；$(0.6,0,0.8)$、$D=90$ 处的位移：16.06 / 21.81 / 37.27 像素（水 / 冕牌 / 钻石）；红蓝位移差 0.48 像素（BK7）vs 7.0 像素（默认 0.15）。

## 9. 参考文献与致谢

1. **m2-md, *liquid-glass-refraction-shader***（MIT）——光学/SDF 数学及 thickness/bevel/refraction/blur/specular uniform 划分的正源；`optics.rs` 与 `sdf2d.rs` 是其 `optics.ts` / `sdf2d.ts` 的移植（含测试）。 <https://github.com/m2-md/liquid-glass-refraction-shader>
2. **jeantimex, *glass-effect-webgpu***（MIT）——WGSL 管线结构（uniform 布局、合成 pass 组织）。 <https://github.com/jeantimex/glass-effect-webgpu>
3. I. Quilez, *2D distance functions*——圆角盒 SDF 精确公式。 <https://iquilezles.org/articles/distfunctions/>
4. C. Schlick, "An Inexpensive BRDF Model for Physically-based Rendering", *Computer Graphics Forum* 13(3), 1994——式 (5.2)。
5. *RefractiveIndex.INFO*——折射率表值。 <https://refractiveindex.info/>
6. **heonny, *egui-glass***（live-backdrop 离屏管线，P2 参考）；**zaroutt, *Niri-glass*** 与 **OverShifted, *LiquidGlass***（合成器级 SDF 折射参考）。

以上项目均为 MIT 许可；Vitreo 亦为 MIT 许可（见 `LICENSE`）。本文档与全部图像为 Vitreo 项目原创工作。

---

*Vitreo —— write once, refract everywhere.*
