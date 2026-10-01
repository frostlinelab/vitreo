//! 弹簧积分器 —— 果冻动画的状态来源。
//!
//! 半隐式 Euler（先更新速度、再更新位置），固定子步长保证大 `dt` 下的稳定。
//! 位置弹簧的 `velocity` 直接就是 [`crate::AnimatedPanel`] 喂给
//! `GlassPanel::velocity` 的果冻形变驱动量：弹簧本身在欠阻尼下振荡，
//! 着色器端自然得到随时间摆动的形变，无需额外的振荡器。

/// 弹簧参数（质量-弹簧-阻尼）。
///
/// 预设与 iOS `UISpringTimingResponse` 的常用等效值一致：
/// - [`SpringConfig::smooth`]：临界阻尼，无过冲，约 0.35 s 收敛；
/// - [`SpringConfig::bouncy`]：欠阻尼（ζ ≈ 0.5），带一次可见过冲的果冻感。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringConfig {
    /// 刚度 k（1/s²）。越大回弹越快。
    pub stiffness: f32,
    /// 阻尼系数 c（1/s）。临界阻尼值为 `2 * sqrt(k * m)`。
    pub damping: f32,
    /// 质量 m。归一为 1，一般不用动。
    pub mass: f32,
}

impl SpringConfig {
    /// 自定义参数。
    pub const fn new(stiffness: f32, damping: f32) -> Self {
        Self { stiffness, damping, mass: 1.0 }
    }

    /// 临界阻尼：最快且无过冲。
    pub const fn smooth() -> Self {
        Self::new(170.0, 26.0)
    }

    /// 欠阻尼（ζ ≈ 0.5）：带果冻感的回弹。
    pub const fn bouncy() -> Self {
        Self::new(170.0, 13.0)
    }

    /// 阻尼比 ζ = c / (2√(km))：<1 欠阻尼（过冲），=1 临界，>1 过阻尼。
    pub fn damping_ratio(&self) -> f32 {
        self.damping / (2.0 * (self.stiffness * self.mass).sqrt())
    }
}

impl Default for SpringConfig {
    fn default() -> Self {
        Self::smooth()
    }
}

/// 一维弹簧（值 + 目标 + 速度）。二维位置用两颗并排的 [`Spring`] 表达。
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    value: f32,
    target: f32,
    velocity: f32,
    config: SpringConfig,
}

/// 子步长（秒）：1/120 s 内积分足够精确，大于它的帧被切成若干子步。
const SUBSTEP: f32 = 1.0 / 120.0;
/// 单帧 `dt` 上限：丢帧（切后台等）时不让弹簧爆掉。
const MAX_DT: f32 = 1.0 / 15.0;

impl Spring {
    /// 从 `value` 出发、目标也是 `value` 的静止弹簧。
    pub fn new(value: f32, config: SpringConfig) -> Self {
        Self { value, target: value, velocity: 0.0, config }
    }

    /// 设置目标。速度保持不变——这正是果冻感的来源。
    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// 瞬移到 `value`：值、目标、速度全部重置（程序化摆放、DPI 缩放时用）。
    pub fn jump_to(&mut self, value: f32) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
    }

    /// 推进 `dt` 秒。内部按 [`SUBSTEP`] 切子步，`dt` 被钳制到 [`MAX_DT`]。
    pub fn advance(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, MAX_DT);
        if dt <= 0.0 {
            return;
        }
        let steps = (dt / SUBSTEP).ceil().max(1.0) as u32;
        let h = dt / steps as f32;
        let k = self.config.stiffness;
        let c = self.config.damping;
        let m = self.config.mass;
        for _ in 0..steps {
            // 半隐式 Euler：先速度后位置，能量耗散方向正确，不会越振越大。
            let accel = (k * (self.target - self.value) - c * self.velocity) / m;
            self.velocity += accel * h;
            self.value += self.velocity * h;
        }
    }

    /// 当前值。
    pub fn value(&self) -> f32 {
        self.value
    }

    /// 当前速度（px/s）——位置弹簧的读数即 `GlassPanel::velocity`。
    pub fn velocity(&self) -> f32 {
        self.velocity
    }

    /// 当前目标。
    pub fn target(&self) -> f32 {
        self.target
    }

    /// 是否已收敛（距目标 < 0.1 px 且速度 < 1 px/s）。
    /// 可用于"静止时跳过重绘"之类的优化。
    pub fn settled(&self) -> bool {
        (self.target - self.value).abs() < 0.1 && self.velocity.abs() < 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn advance_seconds(s: &mut Spring, seconds: f32, fps: f32) {
        let dt = 1.0 / fps;
        let steps = (seconds / dt).ceil() as u32;
        for _ in 0..steps {
            s.advance(dt);
        }
    }

    #[test]
    fn converges_to_target_from_arbitrary_start() {
        let mut s = Spring::new(0.0, SpringConfig::smooth());
        s.set_target(800.0);
        advance_seconds(&mut s, 2.0, 60.0);
        assert!((s.value() - 800.0).abs() < 0.5, "value = {}", s.value());
        assert!(s.settled());
    }

    #[test]
    fn critically_damped_never_overshoots() {
        let mut s = Spring::new(0.0, SpringConfig::smooth());
        s.set_target(100.0);
        let mut crossed = false;
        for _ in 0..240 {
            s.advance(1.0 / 120.0);
            if s.value() > 100.0 {
                crossed = true;
            }
        }
        assert!(!crossed, "临界阻尼不应越过目标，value = {}", s.value());
    }

    #[test]
    fn underdamped_overshoots_at_least_once() {
        let mut s = Spring::new(0.0, SpringConfig::bouncy());
        s.set_target(100.0);
        let mut crossed = false;
        for _ in 0..240 {
            s.advance(1.0 / 120.0);
            if s.value() > 100.5 {
                crossed = true;
            }
        }
        assert!(crossed, "欠阻尼应至少过冲一次");
        assert!(s.settled(), "但最终仍要收敛");
    }

    #[test]
    fn large_dt_is_stable() {
        // 丢帧模拟：每帧 1/15 s（MAX_DT 上限），连续 60 帧不得发散。
        let mut s = Spring::new(0.0, SpringConfig::bouncy());
        s.set_target(500.0);
        for _ in 0..60 {
            s.advance(1.0 / 15.0);
        }
        assert!(s.value().is_finite());
        assert!((s.value() - 500.0).abs() < 1.0, "value = {}", s.value());
    }

    #[test]
    fn jump_to_resets_velocity_and_target() {
        let mut s = Spring::new(0.0, SpringConfig::bouncy());
        s.set_target(1000.0);
        s.advance(0.05);
        s.jump_to(42.0);
        assert_eq!(s.value(), 42.0);
        assert_eq!(s.target(), 42.0);
        assert_eq!(s.velocity(), 0.0);
    }

    #[test]
    fn velocity_starts_at_target_speed() {
        // 一阶近似：释放瞬间加速度 ≈ k * 距离，速度 ≈ a * dt。
        let mut s = Spring::new(0.0, SpringConfig::smooth());
        s.set_target(400.0);
        s.advance(1.0 / 120.0);
        let expected_accel = 170.0 * 400.0;
        let expected_v = expected_accel * (1.0 / 120.0);
        assert!((s.velocity() - expected_v).abs() / expected_v < 1e-4);
    }

    #[test]
    fn deterministic_replay() {
        let run = || {
            let mut s = Spring::new(10.0, SpringConfig::bouncy());
            s.set_target(-30.0);
            for i in 0..300 {
                s.advance(1.0 / 60.0 + (i % 7) as f32 * 1e-4);
            }
            s.value()
        };
        assert!((run() - run()).abs() < 1e-6);
    }

    #[test]
    fn damping_ratio_matches_presets() {
        assert!((SpringConfig::smooth().damping_ratio() - 1.0).abs() < 0.02);
        assert!((SpringConfig::bouncy().damping_ratio() - 0.5).abs() < 0.02);
    }
}
