//! Inlines → Pango markup for one `GtkLabel`. Pure, so it is tested
//! without a display.
//!
//! Card text is untrusted: every piece of it goes through
//! [`glib::markup_escape_text`], and the only tags are the ones written
//! here. There is no `<a>`: a link is underlined text whose URL the label
//! shows as a tooltip ([`link_at`]), never something to click.

use std::fmt::Write;
use std::ops::Range;

use cards_core::content::{ImageTarget, Inline};
use gtk::glib;

/// A label's markup and where its links are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Markup {
    /// Pango markup: escaped text inside our own tags.
    pub markup: String,
    /// Each link's byte range in the label's text (the markup without its
    /// tags, as `pango::Layout` indexes it) and its URL.
    pub links: Vec<(Range<usize>, String)>,
}

impl Markup {
    /// Appends `text`, escaped.
    fn text(&mut self, text: &str, plain_len: &mut usize) {
        self.markup.push_str(&glib::markup_escape_text(text));
        *plain_len += text.len();
    }
}

/// The markup of `inlines`.
pub fn inline_markup(inlines: &[Inline]) -> Markup {
    let mut out = Markup::default();
    let mut plain_len = 0;
    push(&mut out, inlines, &mut plain_len);
    out
}

fn push(out: &mut Markup, inlines: &[Inline], len: &mut usize) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => out.text(text, len),
            Inline::Emphasis(inner) => tagged(out, "i", inner, len),
            Inline::Strong(inner) => tagged(out, "b", inner, len),
            Inline::Strikethrough(inner) => tagged(out, "s", inner, len),
            Inline::Code(code) => tagged_text(out, "tt", code, len),
            // Shown as its source until math rendering comes (SPEC §4.2's
            // fallback).
            Inline::Math { source, .. } => tagged_text(out, "tt", source, len),
            Inline::Link { content, url } => {
                let start = *len;
                tagged(out, "u", content, len);
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

fn tagged(out: &mut Markup, tag: &str, inner: &[Inline], len: &mut usize) {
    let _ = write!(out.markup, "<{tag}>");
    push(out, inner, len);
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
    use cards_core::content::{Block, parse};
    use gtk::pango;

    use super::*;

    /// The markup of a one-paragraph document.
    fn markup(md: &str) -> Markup {
        match parse(md).blocks.as_slice() {
            [Block::Paragraph(inlines)] => inline_markup(inlines),
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
            "<i>a</i> <b>b</b> <s>c</s> <tt>d</tt> <tt>e</tt>"
        );
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
                    shown(&inline_markup(&inlines).markup);
                }
            }
        }
    }
}
