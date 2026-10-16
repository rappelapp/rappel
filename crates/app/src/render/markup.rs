//! Inlines → Pango markup for one `GtkLabel`. Pure, so it is tested
//! without a display.
//!
//! Card text is untrusted: every piece of it goes through
//! [`glib::markup_escape_text`], and the only tags are the ones written
//! here. There is no `<a>`: a link is underlined text whose URL the label
//! shows as a tooltip ([`link_at`]), never something to click.

use std::fmt::Write;
use std::ops::Range;

use gtk::glib;
use rappel_core::content::{ImageTarget, Inline};

use super::math::{Budget, Placement};

/// Stands for a drawn formula in the label's text: Pango's object
/// replacement character. The paragraph gives it a box of the formula's
/// size (`math_paragraph`).
pub const PLACEHOLDER: char = '\u{FFFC}';

/// A label's markup and where its links are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Markup {
    /// Pango markup: escaped text inside our own tags.
    #[expect(
        clippy::struct_field_names,
        reason = "it is the markup string; any other name would hide that"
    )]
    pub markup: String,
    /// Each link's byte range in the label's text (the markup without its
    /// tags, as `pango::Layout` indexes it) and its URL.
    pub links: Vec<(Range<usize>, String)>,
    /// The formulas to draw, in order: one per [`PLACEHOLDER`], at that byte
    /// index of the label's text.
    pub formulas: Vec<Formula>,
}

/// A formula drawn in the text flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formula {
    /// Where its [`PLACEHOLDER`] is, in the label's text.
    pub index: usize,
    pub source: String,
    /// `$$…$$` where it cannot be a block: the display style.
    pub display: bool,
}

impl Markup {
    /// Appends `text`, escaped.
    fn text(&mut self, text: &str, plain_len: &mut usize) {
        self.markup.push_str(&glib::markup_escape_text(text));
        *plain_len += text.len();
    }
}

/// The markup of `inlines`. `math` decides each formula, in document order:
/// drawn (a placeholder), or its source in monospace, with a wavy underline
/// if the LaTeX is invalid.
pub fn inline_markup(inlines: &[Inline], math: &mut Budget) -> Markup {
    let mut out = Markup::default();
    let mut plain_len = 0;
    push(&mut out, inlines, &mut plain_len, math);
    out
}

fn push(out: &mut Markup, inlines: &[Inline], len: &mut usize, math: &mut Budget) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => out.text(text, len),
            Inline::Emphasis(inner) => tagged(out, "i", inner, len, math),
            Inline::Strong(inner) => tagged(out, "b", inner, len, math),
            Inline::Strikethrough(inner) => tagged(out, "s", inner, len, math),
            Inline::Code(code) => tagged_text(out, "tt", code, len),
            Inline::Math { source, display } => match math.place(source, *display) {
                Placement::Formula => {
                    out.formulas.push(Formula {
                        index: *len,
                        source: source.clone(),
                        display: *display,
                    });
                    out.markup.push(PLACEHOLDER);
                    *len += PLACEHOLDER.len_utf8();
                }
                // Pango's own "error" underline (a wavy line, drawn in a
                // colour Pango picks): markup cannot use a CSS class.
                Placement::ErrorSource => {
                    out.markup.push_str("<span underline=\"error\">");
                    tagged_text(out, "tt", source, len);
                    out.markup.push_str("</span>");
                }
                Placement::Source => tagged_text(out, "tt", source, len),
            },
            Inline::Link { content, url } => {
                let start = *len;
                tagged(out, "u", content, len, math);
                out.links.push((start..*len, url.clone()));
            }
            Inline::Image { alt, target } => match target {
                ImageTarget::Media(_) => {
                    let text = if alt.is_empty() {
                        "[image]".to_owned()
                    } else {
                        format!("[image: {alt}]")
                    };
                    tagged_text(out, "i", &text, len);
                }
                // Never fetched: shown as the Markdown that was typed.
                ImageTarget::Other(url) => out.text(&format!("![{alt}]({url})"), len),
            },
            Inline::LineBreak => out.text("\n", len),
        }
    }
}

fn tagged(out: &mut Markup, tag: &str, inner: &[Inline], len: &mut usize, math: &mut Budget) {
    let _ = write!(out.markup, "<{tag}>");
    push(out, inner, len, math);
    let _ = write!(out.markup, "</{tag}>");
}

fn tagged_text(out: &mut Markup, tag: &str, text: &str, len: &mut usize) {
    let _ = write!(out.markup, "<{tag}>");
    out.text(text, len);
    let _ = write!(out.markup, "</{tag}>");
}

/// The URL of the link at byte `index` of the label's text, if any.
pub fn link_at(links: &[(Range<usize>, String)], index: usize) -> Option<&str> {
    links
        .iter()
        .find(|(range, _)| range.contains(&index))
        .map(|(_, url)| url.as_str())
}

#[cfg(test)]
mod tests {
    use gtk::pango;
    use rappel_core::content::{Block, parse};

    use super::*;

    /// The markup of a one-paragraph document.
    fn markup(md: &str) -> Markup {
        match parse(md).blocks.as_slice() {
            [Block::Paragraph(inlines)] => inline_markup(inlines, &mut Budget::default()),
            other => panic!("not one paragraph: {other:?}"),
        }
    }

    /// The text Pango shows for `markup`, which must be valid.
    fn shown(markup: &str) -> String {
        let (_, text, _) = pango::parse_markup(markup, '\0').expect("valid markup");
        text.to_string()
    }

    #[test]
    fn formatting_uses_our_tags() {
        assert_eq!(
            markup("*a* **b** ~~c~~ `d` $e$").markup,
            "<i>a</i> <b>b</b> <s>c</s> <tt>d</tt> \u{FFFC}"
        );
    }

    #[test]
    fn a_drawn_formula_is_a_placeholder_at_its_byte_index() {
        let out = markup("é $x^2$ à *$\\frac{1}{2}$* [l $y$](u) fin");
        let text = shown(&out.markup);
        assert_eq!(text, "é \u{FFFC} à \u{FFFC} l \u{FFFC} fin");
        assert_eq!(out.formulas.len(), 3);
        for formula in &out.formulas {
            assert!(
                text[formula.index..].starts_with(PLACEHOLDER),
                "{formula:?}"
            );
            assert!(!formula.display);
        }
        assert_eq!(out.formulas[0].source, "x^2");
        assert_eq!(out.formulas[1].source, "\\frac{1}{2}");
        // The link covers its text, the formula in it included.
        let (range, url) = &out.links[0];
        assert_eq!(url, "u");
        assert_eq!(&text[range.clone()], "l \u{FFFC}");
        assert_eq!(out.formulas[2].index, range.end - PLACEHOLDER.len_utf8());
    }

    #[test]
    fn invalid_latex_is_source_with_an_error_underline() {
        let out = markup("a $\\notacommand{x}$ b");
        assert_eq!(out.formulas, []);
        assert_eq!(
            out.markup,
            "a <span underline=\"error\"><tt>\\notacommand{x}</tt></span> b"
        );
        assert_eq!(shown(&out.markup), "a \\notacommand{x} b");
    }

    #[test]
    fn formulas_past_the_budget_are_plain_source() {
        let md = "$x$ ".repeat(super::super::math::MAX_FORMULAS + 2);
        let out = markup(&md);
        assert_eq!(out.formulas.len(), super::super::math::MAX_FORMULAS);
        assert!(
            out.markup.ends_with("<tt>x</tt> <tt>x</tt>"),
            "{}",
            &out.markup[out.markup.len() - 40..]
        );
    }

    #[test]
    fn a_source_that_could_break_the_markup_stays_escaped() {
        let out = markup("$<b>&</b>$ $\\frac{<i>x$");
        shown(&out.markup);
        assert!(!out.markup.contains("<i>"), "{}", out.markup);
    }

    #[test]
    fn card_text_is_escaped() {
        let md = r#"<b>x</b> & <span foreground="red">y</span> &amp; `<i>` 'q' "q""#;
        let out = markup(md);
        assert!(!out.markup.contains("<b>"), "{}", out.markup);
        assert!(!out.markup.contains("<span"), "{}", out.markup);
        assert_eq!(
            shown(&out.markup),
            r#"<b>x</b> & <span foreground="red">y</span> & <i> 'q' "q""#
        );
    }

    #[test]
    fn links_are_underlined_with_their_range() {
        let out = markup("é [lïnk *x*](https://a.org/?q=<1>&r) [b](c) end");
        assert!(!out.markup.contains("<a"));
        let text = shown(&out.markup);
        assert_eq!(text, "é lïnk x b end");
        assert_eq!(out.links.len(), 2);
        let (first, url) = &out.links[0];
        assert_eq!(&text[first.clone()], "lïnk x");
        assert_eq!(url, "https://a.org/?q=<1>&r");
        assert_eq!(&text[out.links[1].0.clone()], "b");
        assert_eq!(
            link_at(&out.links, first.start),
            Some("https://a.org/?q=<1>&r")
        );
        assert_eq!(
            link_at(&out.links, first.end - 1),
            Some("https://a.org/?q=<1>&r")
        );
        assert_eq!(link_at(&out.links, first.end), None);
        assert_eq!(link_at(&out.links, 0), None);
    }

    #[test]
    fn images_are_placeholders_or_text() {
        assert_eq!(
            shown(&markup("![a <cat>](media:ab.png) ![](media:c.png)").markup),
            "[image: a <cat>] [image]"
        );
        assert_eq!(
            shown(&markup("![x](https://e.org/i.png)").markup),
            "![x](https://e.org/i.png)"
        );
    }

    #[test]
    fn line_breaks_are_newlines() {
        assert_eq!(shown(&markup("a  \nb").markup), "a\nb");
    }

    #[test]
    fn markup_is_always_valid() {
        for md in [
            "<<>>&&;;'\"",
            "\u{1}\u{8}\u{b}\u{1f}\u{7f}",
            "**[`<`](>)**",
            "a\0b",
            "&#0; &#xD800; &lt;",
        ] {
            for block in parse(md).blocks {
                if let Block::Paragraph(inlines) = block {
                    shown(&inline_markup(&inlines, &mut Budget::default()).markup);
                }
            }
        }
    }
}
