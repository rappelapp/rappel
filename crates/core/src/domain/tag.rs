//! Tags: flat labels on cards, unique by their name key (ADR 0029).

use super::has_nul;
use crate::error::{Error, Result};

/// The longest tag name, in UTF-8 bytes once trimmed.
pub const MAX_TAG_NAME_BYTES: usize = 256;

/// A tag. Tags are never deleted: one no card uses any more stays, so the
/// same name later gets the same id back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    /// Tag id (`UUIDv7` string).
    pub id: String,
    /// The name, trimmed, unique among tags by its
    /// [`name_key`](super::name_key).
    pub name: String,
}

/// `name` trimmed, if it can be a tag name: not empty, at most
/// [`MAX_TAG_NAME_BYTES`], no NUL and no comma. The card editor's tags
/// field is a comma-separated list (SPEC §4.4), so a comma inside a name
/// could not be typed back.
///
/// # Errors
/// [`Error::EmptyTagName`], [`Error::TagNameTooLong`],
/// [`Error::TagNameHasComma`], [`Error::ContainsNul`].
pub fn check_tag_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        Err(Error::EmptyTagName)
    } else if has_nul(name) {
        Err(Error::ContainsNul)
    } else if name.contains(',') {
        Err(Error::TagNameHasComma)
    } else if name.len() > MAX_TAG_NAME_BYTES {
        Err(Error::TagNameTooLong {
            limit: MAX_TAG_NAME_BYTES,
        })
    } else {
        Ok(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_names_are_trimmed_and_checked() {
        assert_eq!(check_tag_name("  maths \n").unwrap(), "maths");
        assert!(matches!(check_tag_name(" \t"), Err(Error::EmptyTagName)));
        assert!(matches!(check_tag_name("a,b"), Err(Error::TagNameHasComma)));
        assert!(matches!(check_tag_name("a\0b"), Err(Error::ContainsNul)));
        let longest = "é".repeat(MAX_TAG_NAME_BYTES / 2);
        assert_eq!(check_tag_name(&longest).unwrap(), longest);
        assert!(matches!(
            check_tag_name(&format!("{longest}x")),
            Err(Error::TagNameTooLong { limit: 256 })
        ));
        // Trimmed before measuring.
        assert!(check_tag_name(&format!(" {longest} ")).is_ok());
    }
}
