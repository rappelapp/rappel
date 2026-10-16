//! The rendering decisions that need no GTK: how a side is laid out, the
//! style class of a heading, a list item's marker. Pure and tested.

use cards_core::content::{Block, Document};

/// Which side of a card is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Front,
    Back,
}

/// How a side is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// One paragraph: large and centred, as cards looked before Markdown
    /// (`title-2` on the front, `title-3` on the back).
    Simple(Side),
    /// Anything else: body text, aligned to the start.
    Rich,
}

impl Style {
    /// The style for `doc` shown on `side`.
    pub fn of(doc: &Document, side: Side) -> Self {
        match doc.blocks.as_slice() {
            [Block::Paragraph(_)] => Self::Simple(side),
            _ => Self::Rich,
        }
    }

    /// The style class of a paragraph's label, if any.
    pub fn paragraph_class(self) -> Option<&'static str> {
        match self {
            Self::Simple(Side::Front) => Some("title-2"),
            Self::Simple(Side::Back) => Some("title-3"),
            Self::Rich => None,
        }
    }

    /// Whether text is centred.
    pub fn centred(self) -> bool {
        matches!(self, Self::Simple(_))
    }
}

/// Whether the back is shown after the reveal: not when there is nothing
/// to show, so only the grade buttons appear.
pub fn back_shown(back: &Document) -> bool {
    !back.is_empty()
}

/// The libadwaita style class of a heading of `level` (1 to 6).
pub fn heading_class(level: u8) -> &'static str {
    match level {
        0 | 1 => "title-1",
        2 => "title-2",
        3 => "title-3",
        4 => "title-4",
        _ => "heading",
    }
}

/// The marker of item `index` (from 0) of a list `depth` lists deep (from
/// 0): a bullet that changes with depth, or a number from `start`.
pub fn list_marker(start: Option<u64>, index: usize, depth: usize) -> String {
    match start {
        Some(start) => {
            let index = u64::try_from(index).unwrap_or(u64::MAX);
            format!("{}.", start.saturating_add(index))
        }
        None => ["•", "◦", "▪"][depth % 3].to_owned(),
    }
}

/// How many widgets [`super::render`] makes for `block`, to stay within
/// its budget.
pub fn widget_cost(block: &Block) -> usize {
    match block {
        Block::Paragraph(_) | Block::Heading { .. } | Block::Rule | Block::DisplayMath(_) => 1,
        // A label in a box.
        Block::CodeBlock { .. } => 2,
        // A grid; per item a marker and a box with the item's blocks.
        Block::List { items, .. } => {
            1 + items
                .iter()
                .map(|item| 2 + item.iter().map(widget_cost).sum::<usize>())
                .sum::<usize>()
        }
        // A box, a separator and a box with the blocks.
        Block::BlockQuote(blocks) => 3 + blocks.iter().map(widget_cost).sum::<usize>(),
        // A scroller, a grid and a label per cell.
        Block::Table { header, rows, .. } => {
            2 + header.len() + rows.iter().map(Vec::len).sum::<usize>()
        }
    }
}

#[cfg(test)]
mod tests {
    use cards_core::content::parse;

    use super::*;

    #[test]
    fn one_paragraph_is_simple_and_the_rest_rich() {
        let style = |md| Style::of(&parse(md), Side::Front);
        assert_eq!(style("What is *x*?"), Style::Simple(Side::Front));
        assert_eq!(
            style("line one\nline two  \nthree"),
            Style::Simple(Side::Front)
        );
        assert_eq!(style("one\n\ntwo"), Style::Rich);
        assert_eq!(style("# Title"), Style::Rich);
        assert_eq!(style("- item"), Style::Rich);
        assert_eq!(style("$$x$$"), Style::Rich);
        assert_eq!(style(""), Style::Rich);
        assert_eq!(
            Style::of(&parse("x"), Side::Back),
            Style::Simple(Side::Back)
        );
    }

    #[test]
    fn simple_sides_keep_the_old_look() {
        assert_eq!(
            Style::Simple(Side::Front).paragraph_class(),
            Some("title-2")
        );
        assert_eq!(Style::Simple(Side::Back).paragraph_class(), Some("title-3"));
        assert_eq!(Style::Rich.paragraph_class(), None);
        assert!(Style::Simple(Side::Back).centred());
        assert!(!Style::Rich.centred());
    }

    #[test]
    fn an_empty_back_is_not_shown() {
        assert!(!back_shown(&parse("")));
        assert!(!back_shown(&parse(" \n\n\t")));
        assert!(back_shown(&parse("x")));
        assert!(back_shown(&parse("---")));
    }

    #[test]
    fn headings_use_title_classes() {
        let classes: Vec<_> = (1..=6).map(heading_class).collect();
        assert_eq!(
            classes,
            [
                "title-1", "title-2", "title-3", "title-4", "heading", "heading"
            ]
        );
    }

    #[test]
    fn widget_costs() {
        let cost = |md| parse(md).blocks.iter().map(widget_cost).sum::<usize>();
        assert_eq!(cost("a\n\nb"), 2);
        assert_eq!(cost("```\nx\n```"), 2);
        assert_eq!(cost("- a\n- b"), 1 + 3 + 3);
        assert_eq!(cost("> a\n>\n> b"), 3 + 2);
        assert_eq!(cost("| a | b |\n|--|--|\n| 1 | 2 |\n| 3 |"), 2 + 2 + 2 + 2);
        // A huge list is costly, so it is shown as plain text.
        assert!(cost(&"- a\n".repeat(10_000)) > 20_000);
    }

    #[test]
    fn list_markers() {
        assert_eq!(list_marker(None, 0, 0), "•");
        assert_eq!(list_marker(None, 5, 1), "◦");
        assert_eq!(list_marker(None, 0, 2), "▪");
        assert_eq!(list_marker(None, 0, 3), "•");
        assert_eq!(list_marker(Some(1), 0, 0), "1.");
        assert_eq!(list_marker(Some(3), 2, 4), "5.");
        assert_eq!(list_marker(Some(u64::MAX), 1, 0), format!("{}.", u64::MAX));
    }
}
