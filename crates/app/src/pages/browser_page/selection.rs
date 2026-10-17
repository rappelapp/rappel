//! The cards chosen on the All Cards page. Pure: a set of card ids and
//! whether the page is in selection mode. The page owns it (and not a
//! `GtkMultiSelection`) because a click opens the editor outside selection
//! mode, and a list view with a selection model selects on a click instead
//! (ADR 0036).

use std::collections::HashSet;

#[derive(Debug, Default)]
pub struct Selection {
    active: bool,
    ids: HashSet<String>,
}

impl Selection {
    /// Whether the page is in selection mode.
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn enter(&mut self) {
        self.active = true;
    }

    /// Leaves selection mode and forgets the choice.
    pub fn leave(&mut self) {
        self.active = false;
        self.ids.clear();
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    /// Chooses `id` or, if chosen, drops it. Returns whether it is chosen
    /// now.
    pub fn toggle(&mut self, id: &str) -> bool {
        if self.ids.remove(id) {
            false
        } else {
            self.ids.insert(id.to_owned());
            true
        }
    }

    /// Chooses every one of `ids` (selection mode is on).
    pub fn select_all<'a>(&mut self, ids: impl IntoIterator<Item = &'a str>) {
        self.active = true;
        self.ids.extend(ids.into_iter().map(str::to_owned));
    }

    /// After the list was read again: forgets the cards that are not in it.
    pub fn retain<'a>(&mut self, still_there: impl IntoIterator<Item = &'a str>) {
        let still_there: HashSet<&str> = still_there.into_iter().collect();
        self.ids.retain(|id| still_there.contains(id.as_str()));
    }

    /// The chosen ids among `in_order`, in that order (the order of the
    /// list, which is the order the user sees).
    pub fn ids_in<'a>(&self, in_order: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        in_order
            .into_iter()
            .filter(|id| self.ids.contains(*id))
            .map(str::to_owned)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_chooses_and_drops() {
        let mut s = Selection::default();
        assert!(!s.active());
        s.enter();
        assert!(s.toggle("a"));
        assert!(s.toggle("b"));
        assert!(!s.toggle("a"));
        assert_eq!(
            (s.len(), s.contains("a"), s.contains("b")),
            (1, false, true)
        );
    }

    #[test]
    fn leaving_forgets_the_choice() {
        let mut s = Selection::default();
        s.select_all(["a", "b", "c"]);
        assert!(s.active());
        assert_eq!(s.len(), 3);
        s.leave();
        assert!(!s.active());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn a_new_list_drops_the_cards_that_went() {
        let mut s = Selection::default();
        s.select_all(["a", "b", "c"]);
        s.retain(["b", "c", "d"]);
        assert_eq!(s.ids_in(["a", "b", "c", "d"]), ["b", "c"]);
    }

    #[test]
    fn the_ids_come_in_the_lists_order() {
        let mut s = Selection::default();
        s.select_all(["c", "a"]);
        assert_eq!(s.ids_in(["a", "b", "c"]), ["a", "c"]);
    }
}
