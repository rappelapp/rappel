//! Drop-downs over a list of names.

use gtk::prelude::*;

/// Shows `labels` in `dropdown`, as plain text (they are the user's deck and
/// tag names), and makes the search in its popup work: `GtkDropDown` searches
/// by `expression`, which a model set by hand does not have
/// (`GtkDropDown:enable-search`).
pub fn set_labels(dropdown: &gtk::DropDown, labels: &[String]) {
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    dropdown.set_model(Some(&gtk::StringList::new(&refs)));
    dropdown.set_expression(Some(gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        None::<gtk::Expression>,
        "string",
    )));
}
