//! Hashtag identity: what two spellings of a tag have to agree on.
//!
//! Lives at the crate root rather than under `render` or `db` because all three
//! layers need the same answer. The renderer decides a run of text is a tag and
//! writes the link; the database stores it; the templates build links back to it.
//! If any two of those disagreed about case or escaping, a tag would render as a
//! link to a page that does not list the post the link came from.

/// The form a tag is stored and matched under.
///
/// Full Unicode lowercasing, not SQLite's `COLLATE NOCASE` — that folds ASCII
/// only, so `#Café` and `#café` would be two different tags in the same table.
pub fn canonical(name: &str) -> String {
    name.to_lowercase()
}

/// The path a tag links to, percent-encoded.
///
/// Tag characters are letters, digits, `_` and `-` (see `render::markdown`), so
/// only the non-ASCII ones ever need encoding — but they do need it, and a bare
/// `#日本語` in an href would otherwise be a link that some clients mangle and
/// others do not. axum's `Path` extractor decodes on the way back in.
pub fn href(name: &str) -> String {
    format!("/t/{}", crate::url::encode_component(&canonical(name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalization_folds_case_beyond_ascii() {
        assert_eq!(canonical("TypeScript"), "typescript");
        // The case SQLite's NOCASE gets wrong, and the reason this is in Rust.
        assert_eq!(canonical("Café"), canonical("CAFÉ"));
    }

    #[test]
    fn hrefs_are_lowercased_and_percent_encoded() {
        assert_eq!(href("Rust"), "/t/rust");
        assert_eq!(href("web-dev"), "/t/web-dev");
        // Two bytes in UTF-8, so two escapes.
        assert_eq!(href("é"), "/t/%C3%A9");
    }
}
