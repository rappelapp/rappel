//! Objects from the `.ui` files compiled from `data/*.blp`, for dialogs
//! that are not composite templates (alert dialogs).

use gtk::glib;
use gtk::prelude::*;

/// The gresource prefix of every `.ui` file (`data/resources.gresource.xml`).
const RESOURCE_PREFIX: &str = "/io/github/rsmirke/Cards/";

/// A builder over the bundled `file`, e.g. `"restore_dialog.ui"`.
pub fn builder(file: &str) -> gtk::Builder {
    gtk::Builder::from_resource(&format!("{RESOURCE_PREFIX}{file}"))
}

/// Object `id` of `builder` (loaded from `file`), or `None` after logging
/// that it is missing: a bug in the Blueprint file.
pub fn object<T: IsA<glib::Object>>(builder: &gtk::Builder, file: &str, id: &str) -> Option<T> {
    let object = builder.object::<T>(id);
    if object.is_none() {
        tracing::error!("{file} has no {id} object");
    }
    object
}

/// The `AdwAlertDialog` `id` of `file`, when it is the only object needed.
pub fn alert(file: &str, id: &str) -> Option<adw::AlertDialog> {
    object(&builder(file), file, id)
}
