//! [`Callback`]: a closure kept in a widget's template struct.

use std::fmt;
use std::rc::Rc;

/// A shared closure, e.g. `Callback<dyn Fn(Saved)>`. Template structs
/// derive `Debug`, which closures lack; this prints `Callback`.
pub struct Callback<F: ?Sized>(Rc<F>);

impl<F: ?Sized> Callback<F> {
    /// Wraps `f`, e.g. `Callback::new(Rc::new(closure))`.
    pub fn new(f: Rc<F>) -> Self {
        Self(f)
    }

    /// The closure, cloned out so no borrow of its cell is held while it
    /// runs.
    pub fn get(&self) -> Rc<F> {
        Rc::clone(&self.0)
    }
}

impl<F: ?Sized> fmt::Debug for Callback<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Callback")
    }
}
