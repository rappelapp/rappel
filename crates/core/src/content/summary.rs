//! A side's one-line summary with its formulas kept apart, so a card list
//! can draw them ("σ", not "\sigma") while the rest stays plain text
//! (issue #128, ADR 0077). Pure: the app decides how each formula is drawn,
//! and shows the source of one it cannot draw.

use super::plain::{PREVIEW_CHARS, SUMMARY_SOURCE_BYTES};
use super::{Block, Inline, parse};

/// The longest formula source a summary keeps as a formula, in bytes. A
/// longer one is far wider than a row, so it is shown as its (cut) source
/// text, like any other text.
pub(crate) const MAX_SUMMARY_FORMULA_BYTES: usize = 1024;

/// One piece of a [`Summary`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryPiece {
    /// Plain text, already on one line.
    Text(String),
    /// A formula's LaTeX source, without its dollars, to be drawn inline.
    Math(String),
}

/// A side on one line, for a list row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// Text and formulas in order. Spaces between them are in the text
    /// pieces, as the side had them.
    pub pieces: Vec<SummaryPiece>,
    /// The same as plain text, each formula as its source on one line: what
    /// a screen reader is given, and what shows where nothing is drawn.
    pub text: String,
}

/// The summary of the Markdown side `md`: its text without Markdown
/// syntax, every run of whitespace (newlines and block breaks included)
/// as one space, the ends trimmed, and cut once it holds
/// [`PREVIEW_CHARS`] characters (a formula counts as its source). Inline
/// and display formulas become [`SummaryPiece::Math`]: one that starts
/// before the cut is kept whole, one after it is left out like the rest
/// of the text, and one whose source is longer than
/// [`MAX_SUMMARY_FORMULA_BYTES`] is text (cut like any text). Spaces at the
/// ends of a formula's source count as spaces around it, as in
/// [`super::plain_summary`]. Only the first `SUMMARY_SOURCE_BYTES` of `md`
/// are parsed.
#[must_use]
pub fn summary(md: &str) -> Summary {
    let mut end = md.len().min(SUMMARY_SOURCE_BYTES);
    while !md.is_char_boundary(end) {
        end -= 1;
    }
    let doc = parse(&md[..end]);
    let mut out = Builder::default();
    blocks(&doc.blocks, &mut out);
    Summary {
        pieces: out.pieces,
        text: out.text,
    }
}

#[derive(Debug, Default)]
struct Builder {
    pieces: Vec<SummaryPiece>,
    text: String,
    chars: usize,
    /// Whitespace was seen since the last character written.
    space: bool,
    /// The summary is full.
    done: bool,
}

impl Builder {
    fn text(&mut self, text: &str) {
        for c in text.chars() {
            if self.done {
                return;
            }
            if c.is_whitespace() {
                self.space = !self.text.is_empty();
            } else {
                if !self.space_before() {
                    return;
                }
                self.push_char(c);
            }
        }
    }

    /// Writes the pending space, if any; false if the summary is then
    /// full (no space is left at its end).
    fn space_before(&mut self) -> bool {
        if self.space {
            self.space = false;
            if self.chars + 1 >= PREVIEW_CHARS {
                self.done = true;
                return false;
            }
            self.push_char(' ');
        }
        true
    }

    fn push_char(&mut self, c: char) {
        match self.pieces.last_mut() {
            Some(SummaryPiece::Text(text)) => text.push(c),
            _ => self.pieces.push(SummaryPiece::Text(c.to_string())),
        }
        self.text.push(c);
        self.chars += 1;
        if self.chars >= PREVIEW_CHARS {
            self.done = true;
        }
    }

    fn math(&mut self, raw: &str) {
        if self.done {
            return;
        }
        let source = raw.trim();
        // Whitespace at the source's ends is a space around the formula,
        // as the plain summary has it.
        if raw.starts_with(char::is_whitespace) {
            self.space = !self.text.is_empty();
        }
        if source.is_empty() {
            return;
        }
        let space_after = raw.ends_with(char::is_whitespace);
        if source.len() > MAX_SUMMARY_FORMULA_BYTES {
            self.text(raw);
            return;
        }
        if !self.space_before() {
            return;
        }
        self.pieces.push(SummaryPiece::Math(source.to_owned()));
        for (i, word) in source.split_whitespace().enumerate() {
            if i > 0 {
                self.text.push(' ');
                self.chars += 1;
            }
            self.text.push_str(word);
            self.chars += word.chars().count();
        }
        if self.chars >= PREVIEW_CHARS {
            self.done = true;
        }
        if space_after {
            self.space = true;
        }
    }

    /// A line or block break: a space, unless at the start.
    fn line_break(&mut self) {
        self.space = !self.text.is_empty();
    }
}

fn blocks(list: &[Block], out: &mut Builder) {
    for block in list {
        out.line_break();
        match block {
            Block::Paragraph(content) | Block::Heading { content, .. } => inlines(content, out),
            Block::List { items, .. } => {
                for item in items {
                    blocks(item, out);
                }
            }
            Block::CodeBlock { code, .. } => out.text(code),
            Block::BlockQuote(inner) => blocks(inner, out),
            Block::Rule => {}
            Block::Table { header, rows, .. } => {
                for cell in std::iter::once(header).chain(rows).flatten() {
                    out.line_break();
                    inlines(cell, out);
                }
            }
            Block::DisplayMath(source) => out.math(source),
        }
        if out.done {
            return;
        }
    }
}

fn inlines(list: &[Inline], out: &mut Builder) {
    for inline in list {
        match inline {
            Inline::Text(text) | Inline::Code(text) => out.text(text),
            Inline::Math { source, .. } => out.math(source),
            Inline::Emphasis(inner)
            | Inline::Strong(inner)
            | Inline::Strikethrough(inner)
            | Inline::Link { content: inner, .. }
            | Inline::Audio { content: inner, .. } => inlines(inner, out),
            Inline::Image { alt, .. } => out.text(alt),
            Inline::LineBreak => out.line_break(),
        }
        if out.done {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::content::plain_summary;

    fn text(s: &str) -> SummaryPiece {
        SummaryPiece::Text(s.to_owned())
    }

    fn math(s: &str) -> SummaryPiece {
        SummaryPiece::Math(s.to_owned())
    }

    #[test]
    fn formulas_are_kept_apart_with_the_spaces_around_them() {
        let s = summary("Mesure $\\sigma$-finie");
        assert_eq!(s.pieces, [text("Mesure "), math("\\sigma"), text("-finie")]);
        assert_eq!(s.text, "Mesure \\sigma-finie");
        let s = summary("**La tribu** $\\sigma(\\mathcal C)$\n\n$$x^2$$");
        assert_eq!(
            s.pieces,
            [
                text("La tribu "),
                math("\\sigma(\\mathcal C)"),
                text(" "),
                math("x^2")
            ]
        );
        assert_eq!(s.text, "La tribu \\sigma(\\mathcal C) x^2");
    }

    #[test]
    fn without_math_the_text_is_the_plain_summary() {
        for md in [
            "# Title\n\n**bold** and *it* `code` [link](https://x.org)\n\n- one\n- two",
            "  a\n\nb\t c  ",
            "<b>x</b> & y",
            "",
            "| h1 | h2 |\n|--|--|\n| c1 | c2 |",
        ] {
            let s = summary(md);
            assert_eq!(s.text, plain_summary(md), "{md:?}");
            assert!(s.pieces.iter().all(|p| matches!(p, SummaryPiece::Text(_))));
        }
        assert_eq!(summary("").pieces, []);
    }

    #[test]
    fn a_formulas_source_is_one_line_in_the_text_but_kept_as_typed() {
        let s = summary("$$\\begin{aligned}a &= b\\\\\n c &= d\\end{aligned}$$");
        assert_eq!(
            s.pieces,
            [math("\\begin{aligned}a &= b\\\\\n c &= d\\end{aligned}")]
        );
        assert_eq!(s.text, "\\begin{aligned}a &= b\\\\ c &= d\\end{aligned}");
    }

    #[test]
    fn the_summary_is_cut_and_a_formula_past_the_cut_is_left_out() {
        let long = "word ".repeat(100);
        let s = summary(&format!("{long} $x$"));
        assert_eq!(s.text.chars().count(), PREVIEW_CHARS - 1);
        assert!(!s.text.ends_with(' '));
        assert_eq!(s.pieces.len(), 1);
        assert!(s.pieces.iter().all(|p| matches!(p, SummaryPiece::Text(_))));
        // A formula that starts inside is kept whole.
        let s = summary(&format!("{} $\\alpha+\\beta$", "a".repeat(195)));
        assert_eq!(s.pieces.last(), Some(&math("\\alpha+\\beta")));
    }

    #[test]
    fn a_huge_formula_is_cut_text() {
        let source = "x+".repeat(MAX_SUMMARY_FORMULA_BYTES);
        let s = summary(&format!("${source}x$"));
        assert!(matches!(s.pieces.as_slice(), [SummaryPiece::Text(_)]));
        assert_eq!(s.text.chars().count(), PREVIEW_CHARS);
    }

    #[test]
    fn a_formula_of_the_longest_size_is_kept_and_one_byte_more_is_text() {
        let longest = "x".repeat(MAX_SUMMARY_FORMULA_BYTES);
        assert_eq!(summary(&format!("${longest}$")).pieces, [math(&longest)]);
        let over = "x".repeat(MAX_SUMMARY_FORMULA_BYTES + 1);
        assert!(matches!(
            summary(&format!("${over}$")).pieces.as_slice(),
            [SummaryPiece::Text(_)]
        ));
    }

    #[test]
    fn an_image_is_its_alt_text_and_a_line_break_a_space() {
        let s = summary("$a$  \n![a picture](media:x.png)\\\n$b$");
        assert_eq!(s.text, "a a picture b");
        assert_eq!(s.pieces.first(), Some(&math("a")));
        assert_eq!(s.pieces.last(), Some(&math("b")));
    }

    #[test]
    fn an_empty_formula_adds_nothing() {
        let mut out = Builder::default();
        out.text("a");
        out.math("  ");
        out.text(" b");
        assert_eq!(out.pieces, [text("a b")]);
    }

    #[test]
    fn only_a_bounded_prefix_is_parsed() {
        let md = format!("$x$ {}", "word ".repeat(1_000_000));
        let s = summary(&md);
        assert_eq!(s.pieces.first(), Some(&math("x")));
        assert!(s.text.chars().count() <= PREVIEW_CHARS);
        // A cut inside a multibyte character is moved to its start.
        assert_eq!(
            summary(&"é".repeat(20_000)).text.chars().count(),
            PREVIEW_CHARS
        );
    }

    /// What `summary` must hold for any `md`: every formula non-empty,
    /// trimmed and short enough; the text the pieces' text with each
    /// formula's source on one line; at most 200 characters and one
    /// formula; and, unless the cut was reached, the plain summary.
    fn check(md: &str) -> Result<(), TestCaseError> {
        let s = summary(md);
        let mut text = String::new();
        let mut longest = 0;
        for piece in &s.pieces {
            match piece {
                SummaryPiece::Text(t) => text.push_str(t),
                SummaryPiece::Math(source) => {
                    prop_assert!(!source.is_empty());
                    prop_assert_eq!(source.trim(), source.as_str());
                    prop_assert!(source.len() <= MAX_SUMMARY_FORMULA_BYTES);
                    let one_line = source.split_whitespace().collect::<Vec<_>>().join(" ");
                    longest = longest.max(one_line.chars().count());
                    text.push_str(&one_line);
                }
            }
        }
        prop_assert_eq!(&text, &s.text);
        prop_assert!(s.text.chars().count() <= PREVIEW_CHARS + longest);
        if s.text.chars().count() < PREVIEW_CHARS {
            prop_assert_eq!(&s.text, &plain_summary(md));
        }
        Ok(())
    }

    #[test]
    fn odd_dollars_hold_the_rules() {
        for md in [
            "a $x",
            "$$x$",
            "$5-$10",
            "a$ x$b",
            "a$ $b",
            "$x$ $y$",
            "\\$x$",
            "é $σ$ 漢字",
            "$$\n$$",
            "",
        ] {
            check(md).unwrap_or_else(|e| panic!("{md:?}: {e}"));
        }
    }

    proptest! {
        #[test]
        fn any_side_holds_the_rules(md in "[a-zé漢 $\\\\\n{}^_*`-]{0,300}") {
            check(&md)?;
        }

        /// Without a dollar there is no formula, and the text is the
        /// plain summary the rows showed before.
        #[test]
        fn without_dollars_the_text_is_the_plain_summary(md in "[^$]{0,400}") {
            let s = summary(&md);
            prop_assert_eq!(s.text, plain_summary(&md));
            prop_assert!(s.pieces.iter().all(|p| matches!(p, SummaryPiece::Text(_))));
        }
    }
}
