//! 系统 CJK 字体装载 —— 让 egui 面板上的中文正常显示。
//!
//! 按平台候选路径逐个尝试，找到第一颗能读的字体后追加到
//! Proportional / Monospace 两个家族的回退链尾（拉丁字形仍用 egui 默认字体）。
//! 三个示例共用同一份逻辑，故收进绑定 crate。

/// 给 egui 上下文安装系统 CJK 字体（找不到时仅警告，不 panic）。
pub fn install_cjk_fonts(ctx: &egui::Context) {
    let candidates = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("cjk".into(), egui::FontData::from_owned(bytes).into());
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push("cjk".into());
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
    log::warn!("未找到系统 CJK 字体，中文 UI 将显示为方块");
}
