//! Load a Japanese-capable system font as a fallback for egui's defaults.

use std::sync::Arc;

#[cfg(windows)]
const CANDIDATES: &[&str] = &[
    r"C:\Windows\Fonts\YuGothM.ttc",
    r"C:\Windows\Fonts\meiryo.ttc",
    r"C:\Windows\Fonts\msgothic.ttc",
];
#[cfg(not(windows))]
const CANDIDATES: &[&str] = &[
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
];

pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let Some((path, bytes)) = CANDIDATES
        .iter()
        .find_map(|p| std::fs::read(p).ok().map(|b| (*p, b)))
    else {
        log::warn!("no CJK font found; Japanese text will render as boxes");
        return;
    };
    log::info!("using font {path}");
    fonts
        .font_data
        .insert("jp".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("jp".to_owned());
    }
    ctx.set_fonts(fonts);
}
