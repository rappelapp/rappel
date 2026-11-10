//! What a widget's own Pango layout depends on besides its text, shared by
//! `MathParagraph` and `MathLine`, which each keep their last two layouts
//! (ADR 0084: each had a copy of this).

use gtk::prelude::*;

/// The placeholder's length in UTF-8 (3), for Pango's `u32` byte indices.
pub const PLACEHOLDER_BYTES: u32 = 3;

/// How many layouts a widget keeps: GTK alternates between the unlimited
/// width (the horizontal measure) and the real one (drawing). The oldest
/// goes first, with `Vec::remove(0)`: at this size that moves one element
/// (issue #42).
pub const LAYOUT_CACHE_ENTRIES: usize = 2;

/// What a Pango layout depends on besides the text. If it is the same, the
/// layout from the last call is reused: GTK measures and allocates a widget
/// several times and draws it every frame. `css_changed` cannot be
/// overridden from gtk-rs, but a changed font makes GTK measure again, and
/// this comparison then sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutKey {
    width: i32,
    font: String,
    dpi: i32,
    direction: gtk::TextDirection,
}

impl LayoutKey {
    /// The key of `widget`'s layout at `width` pixels (-1: no limit), from
    /// its Pango context, the display's resolution and its direction now.
    pub fn of(widget: &impl IsA<gtk::Widget>, width: i32) -> Self {
        Self {
            width,
            font: widget
                .pango_context()
                .font_description()
                .map(|d| d.to_str().to_string())
                .unwrap_or_default(),
            dpi: widget.settings().gtk_xft_dpi(),
            direction: widget.direction(),
        }
    }

    /// The width the layout is for.
    pub fn width(&self) -> i32 {
        self.width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_placeholder_is_three_bytes() {
        assert_eq!(
            PLACEHOLDER_BYTES as usize,
            super::super::markup::PLACEHOLDER.len_utf8()
        );
    }
}
