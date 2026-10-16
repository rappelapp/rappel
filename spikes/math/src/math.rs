//! LaTeX -> RaTeX display list, the same way `ratex-gtk4` does it.

use ratex_cairo::{CairoOptions, RenderMetrics, measure_display_list};
use ratex_layout::{LayoutOptions, layout, to_display_list, validate_layout};
use ratex_types::{Color, DisplayList, MathStyle};

pub fn display_list(latex: &str, display: bool, color: Color) -> Result<DisplayList, String> {
    let ast = ratex_parser::parse(latex).map_err(|e| format!("Parse error: {e}"))?;
    let style = if display {
        MathStyle::Display
    } else {
        MathStyle::Text
    };
    let opts = LayoutOptions::default().with_style(style).with_color(color);
    let lb = layout(&ast, &opts);
    validate_layout(&lb)?;
    Ok(to_display_list(&lb))
}

pub fn options(font_size: f64, padding: f64) -> CairoOptions {
    CairoOptions {
        font_size,
        padding,
        font_dir: None,
    }
}

pub fn metrics(dl: &DisplayList, font_size: f64, padding: f64) -> RenderMetrics {
    measure_display_list(dl, &options(font_size, padding))
}

pub fn color(rgba: &gtk::gdk::RGBA) -> Color {
    Color::new(rgba.red(), rgba.green(), rgba.blue(), rgba.alpha())
}
