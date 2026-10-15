//! Where the keyboard focus goes in a list after a change. Pure, so it is
//! unit-tested without GTK.

use std::ops::Sub;

/// The row to focus after the row at `removed` was taken out, leaving
/// `remaining` rows: the row now at that position, or the last row if it
/// was the last. `None` when the list is empty: focus the empty state's
/// button instead.
pub fn index_after_removal<T>(removed: T, remaining: T) -> Option<T>
where
    T: Copy + Ord + Sub<Output = T> + From<u8>,
{
    (remaining > T::from(0)).then(|| removed.min(remaining - T::from(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_row_takes_the_place_of_the_removed_one() {
        assert_eq!(index_after_removal(0_usize, 3), Some(0));
        assert_eq!(index_after_removal(1_u32, 3), Some(1));
    }

    #[test]
    fn removing_the_last_row_focuses_the_new_last_row() {
        assert_eq!(index_after_removal(3_usize, 3), Some(2));
        assert_eq!(index_after_removal(1_u32, 1), Some(0));
    }

    #[test]
    fn an_empty_list_has_no_row_to_focus() {
        assert_eq!(index_after_removal(0_usize, 0), None);
        assert_eq!(index_after_removal(5_u32, 0), None);
    }
}
