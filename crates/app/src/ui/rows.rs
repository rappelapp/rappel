//! Row menus. A row's menu items activate the row's own `row.*` actions,
//! which forward to a page action with the row's id as a string, so menu
//! models stay static and live in Blueprint.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

/// Installs page action `name`, which takes an id string and calls `f`
/// with it. Activations without a string are ignored.
pub fn install_id_action<C>(
    klass: &mut C,
    name: &str,
    f: fn(&<<C as ClassStruct>::Type as ObjectSubclass>::Type, &str),
) where
    C: WidgetClassExt,
{
    klass.install_action(name, Some(glib::VariantTy::STRING), move |obj, _, id| {
        if let Some(id) = id.and_then(glib::Variant::str) {
            f(obj, id);
        }
    });
}

/// Activates page action `action` with `id`, from a row's own action.
pub fn forward_id(row: &impl IsA<gtk::Widget>, action: &str, id: &str) {
    if let Err(err) = row.activate_action(action, Some(&id.to_variant())) {
        tracing::error!("cannot activate {action}: {err}");
    }
}

/// Every row has a menu button; its accessible label says which item it
/// is for: "Actions for {name}".
pub fn label_menu_button(button: &gtk::MenuButton, name: &str) {
    button.update_property(&[gtk::accessible::Property::Label(&format!(
        "Actions for {name}"
    ))]);
}
