//! Card content: Markdown parsed into a [`Document`], a block and inline
//! tree that knows nothing about any UI (ADR 0019). The app turns it into
//! widgets; [`plain_text`] and [`plain_summary`] turn it into text.
//!
//! Card text is untrusted. Raw HTML is kept as literal text, never
//! interpreted, and nothing here touches the network or the filesystem.
//! The tree nests at most `MAX_DEPTH` levels, so a hostile card cannot
//! make anything that walks it recurse without bound.

mod parse;
mod plain;

pub use parse::parse;
#[cfg(test)]
use plain::PREVIEW_CHARS;
pub use plain::{one_line_summary, plain_summary, plain_text};

/// How deep containers (block quotes, lists, emphasis, links…) nest at
/// most. Deeper content is flattened into the deepest container, so no
/// text is lost. A list item or a table cell may add up to two levels
/// below this.
pub(crate) const MAX_DEPTH: usize = 16;

/// A parsed card side.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Block>,
}

impl Document {
    /// `true` if there is nothing to show (an empty or blank side).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}

/// A block-level element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    /// `level` is 1 to 6.
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    /// A bullet list (`start` is `None`) or a numbered list starting at
    /// `start`. Each item is a list of blocks, so lists nest.
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    /// `lang` is the first word of a fenced block's info string.
    CodeBlock {
        lang: Option<String>,
        code: String,
    },
    BlockQuote(Vec<Block>),
    /// A thematic break (`---`).
    Rule,
    /// `align` has one entry per column; a row may have fewer cells.
    Table {
        align: Vec<Align>,
        header: Vec<Cell>,
        rows: Vec<Vec<Cell>>,
    },
    /// `$$…$$` on its own in a paragraph: the LaTeX source, without the
    /// dollars.
    DisplayMath(String),
}

/// A table cell.
pub type Cell = Vec<Inline>;

/// The alignment of a table column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

/// An inline element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// Literal text, also for raw HTML, which is never interpreted.
    Text(String),
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Code(String),
    /// A link. The URL is shown, never opened.
    Link {
        content: Vec<Inline>,
        url: String,
    },
    /// `$…$` math (`display: false`), or `$$…$$` where it cannot be a
    /// block (a heading, a table cell). The LaTeX source, without dollars.
    Math {
        source: String,
        display: bool,
    },
    Image {
        alt: String,
        target: ImageTarget,
    },
    /// A hard line break.
    LineBreak,
}

/// Where an image points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageTarget {
    /// `media:<name>`: a file of the app's media store, by name
    /// (`<sha256>.<ext>`). Not checked here: the media store does that.
    Media(String),
    /// Anything else (a remote URL, a path). Never fetched or opened: it
    /// is shown as text (SPEC §4.3).
    Other(String),
}

impl ImageTarget {
    fn from_url(url: &str) -> Self {
        match url.strip_prefix("media:") {
            Some(name) => Self::Media(name.to_owned()),
            None => Self::Other(url.to_owned()),
        }
    }
}

#[cfg(test)]
mod tests;
