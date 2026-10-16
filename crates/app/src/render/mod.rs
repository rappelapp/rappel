//! A card side's [`Document`] → GTK widgets (ADR 0019). Built from data,
//! so this is the one place widgets are made in Rust and not in Blueprint;
//! only libadwaita style classes are used, no colours.
//!
//! Card content is untrusted: text goes into labels either as plain text
//! (markup off) or as markup from [`markup::inline_markup`], which escapes
//! it. No label is selectable or focusable, so the keyboard focus stays on
//! the review page's card area and its keys keep working. Links cannot be
//! clicked; hovering shows their URL.

mod markup;
mod style;

use cards_core::content::{Align, Block, Cell, Document, Inline, plain_text};
use gtk::AccessibleRole as Role;
use gtk::pango;
use gtk::prelude::*;

pub use style::{Side, back_shown};
use style::{Style, heading_class, list_marker, widget_cost};

/// At most this many widgets per side. A hostile card (a 1 MiB list)
/// would otherwise build hundreds of thousands; what does not fit is shown
/// as plain text in one label instead.
const MAX_WIDGETS: usize = 2_000;

/// Space between blocks, in pixels.
const BLOCK_SPACING: i32 = 12;

/// The widgets of `doc` shown on `side`, in a vertical box.
pub fn render(doc: &Document, side: Side) -> gtk::Box {
    let renderer = Renderer {
        style: Style::of(doc, side),
    };
    let container = blocks_box();
    // A block's cost counts everything inside it, so the budget is spent
    // on top-level blocks only.
    let mut budget = MAX_WIDGETS;
    for (i, block) in doc.blocks.iter().enumerate() {
        let cost = widget_cost(block);
        if cost > budget {
            let rest = Document {
                blocks: doc.blocks[i..].to_vec(),
            };
            container.append(&plain_label(&plain_text(&rest), renderer.style.centred()));
            break;
        }
        budget -= cost;
        container.append(&renderer.block(block, 0));
    }
    container
}

#[derive(Debug)]
struct Renderer {
    style: Style,
}

impl Renderer {
    /// `blocks` in a vertical box. `lists` is how many lists deep they are.
    fn blocks(&self, blocks: &[Block], lists: usize) -> gtk::Box {
        let container = blocks_box();
        for block in blocks {
            container.append(&self.block(block, lists));
        }
        container
    }

    fn block(&self, block: &Block, lists: usize) -> gtk::Widget {
        match block {
            Block::Paragraph(inlines) => text_label(
                inlines,
                self.style.centred(),
                self.style.paragraph_class(),
                Role::Label,
            )
            .upcast(),
            Block::Heading { level, content } => {
                let label = text_label(
                    content,
                    self.style.centred(),
                    Some(heading_class(*level)),
                    Role::Heading,
                );
                label.update_property(&[gtk::accessible::Property::Level(i32::from(*level))]);
                label.upcast()
            }
            Block::List { start, items } => self.list(*start, items, lists).upcast(),
            Block::CodeBlock { code, .. } => {
                let label = plain_label(code, false);
                label.add_css_class("monospace");
                label.set_margin_top(12);
                label.set_margin_bottom(12);
                label.set_margin_start(12);
                label.set_margin_end(12);
                let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
                frame.add_css_class("card");
                frame.append(&label);
                frame.upcast()
            }
            Block::BlockQuote(blocks) => {
                let quote = gtk::Box::new(gtk::Orientation::Horizontal, BLOCK_SPACING);
                quote.append(&gtk::Separator::new(gtk::Orientation::Vertical));
                let inner = self.blocks(blocks, lists);
                inner.add_css_class("dim-label");
                inner.set_hexpand(true);
                quote.append(&inner);
                quote.upcast()
            }
            Block::Rule => gtk::Separator::new(gtk::Orientation::Horizontal).upcast(),
            Block::Table {
                align,
                header,
                rows,
            } => table(align, header, rows).upcast(),
            Block::DisplayMath(source) => {
                // The source until math rendering comes (SPEC §4.2's
                // fallback), centred like a formula.
                let label = plain_label(source, true);
                label.add_css_class("monospace");
                label.upcast()
            }
        }
    }

    fn list(&self, start: Option<u64>, items: &[Vec<Block>], lists: usize) -> gtk::Grid {
        let grid = gtk::Grid::builder()
            .column_spacing(8)
            .row_spacing(6)
            .build();
        for (i, item) in items.iter().enumerate() {
            let row = i32::try_from(i).unwrap_or(i32::MAX);
            let marker = gtk::Label::builder()
                .label(list_marker(start, i, lists))
                .xalign(1.0)
                .valign(gtk::Align::Start)
                .build();
            let content = self.blocks(item, lists + 1);
            content.set_hexpand(true);
            grid.attach(&marker, 0, row, 1, 1);
            grid.attach(&content, 1, row, 1, 1);
        }
        grid
    }
}

/// A table in a horizontal scroller, so a wide one fits a narrow window.
fn table(align: &[Align], header: &[Cell], rows: &[Vec<Cell>]) -> gtk::ScrolledWindow {
    let grid = gtk::Grid::builder()
        .column_spacing(18)
        .row_spacing(6)
        .build();
    let all_rows =
        std::iter::once((header, Some("heading"))).chain(rows.iter().map(|r| (r.as_slice(), None)));
    for (r, (cells, class)) in all_rows.enumerate() {
        let row = i32::try_from(r).unwrap_or(i32::MAX);
        for (c, cell) in cells.iter().enumerate() {
            let column = i32::try_from(c).unwrap_or(i32::MAX);
            let label = text_label(cell, false, class, Role::Label);
            // A cell keeps its line; the scroller handles the width.
            label.set_wrap(false);
            label.set_xalign(match align.get(c).copied().unwrap_or(Align::None) {
                Align::None | Align::Left => 0.0,
                Align::Center => 0.5,
                Align::Right => 1.0,
            });
            grid.attach(&label, column, row, 1, 1);
        }
    }
    gtk::ScrolledWindow::builder()
        // Not focusable (it is by default), so the focus stays on the
        // card area; a wide table scrolls with the pointer only.
        .focusable(false)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .child(&grid)
        .build()
}

fn blocks_box() -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, BLOCK_SPACING)
}

/// A wrapping label of `inlines`, with `class` if given.
fn text_label(inlines: &[Inline], centred: bool, class: Option<&str>, role: Role) -> gtk::Label {
    let markup::Markup { markup, links } = markup::inline_markup(inlines);
    let label = wrapping_label(centred, role);
    label.set_use_markup(true);
    label.set_label(&markup);
    if let Some(class) = class {
        label.add_css_class(class);
    }
    if !links.is_empty() {
        show_link_urls(&label, links);
    }
    label
}

/// A wrapping label of plain `text` (markup off: nothing to escape).
fn plain_label(text: &str, centred: bool) -> gtk::Label {
    let label = wrapping_label(centred, Role::Label);
    label.set_use_markup(false);
    label.set_label(text);
    label
}

/// `role` is construct-only, so it is set here: a heading is announced as
/// one (with its level, set by the caller).
fn wrapping_label(centred: bool, role: Role) -> gtk::Label {
    let label = gtk::Label::builder()
        .accessible_role(role)
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .selectable(false)
        .build();
    if centred {
        label.set_justify(gtk::Justification::Center);
        label.set_xalign(0.5);
    } else {
        label.set_xalign(0.0);
    }
    label
}

/// Shows the URL of the link under the pointer as the label's tooltip.
///
/// `query-tooltip` comes when the pointer rests on the widget, with `x`
/// and `y` in the widget's coordinates, and GTK asks again as the pointer
/// moves (gtkwidget.c, `query-tooltip`; gtktooltip.c). In keyboard mode
/// the coordinates are undefined, so no tooltip is offered then.
/// `gtk_label_get_layout_offsets` gives where the label draws its Pango
/// layout, to turn the pointer into a layout position, in Pango units
/// (gtklabel.c).
fn show_link_urls(label: &gtk::Label, links: Vec<(std::ops::Range<usize>, String)>) {
    label.set_has_tooltip(true);
    label.connect_query_tooltip(move |label, x, y, keyboard, tooltip| {
        if keyboard {
            return false;
        }
        let (dx, dy) = label.layout_offsets();
        let (inside, index, _) = label.layout().xy_to_index(
            (x - dx).saturating_mul(pango::SCALE),
            (y - dy).saturating_mul(pango::SCALE),
        );
        let url = usize::try_from(index)
            .ok()
            .filter(|_| inside)
            .and_then(|index| markup::link_at(&links, index));
        match url {
            Some(url) => {
                // Plain text, not markup.
                tooltip.set_text(Some(url));
                true
            }
            None => false,
        }
    });
}
