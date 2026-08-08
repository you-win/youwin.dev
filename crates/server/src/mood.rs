//! The mood a post was written in.
//!
//! A column on `posts` and a picker in the composer, shared by `db`, the
//! authoring API, the export, and the familiar — which is the only thing that
//! reads it, but not the thing that owns it. It lives at the crate root beside
//! [`crate::tag`] for the same reason that does: three layers need to agree on
//! what the values are, and a definition inside any one of them would make the
//! other two depend on it sideways.
//!
//! Storage is the lowercase name, so the column reads as prose in a SQLite
//! shell and the JSON export needs no legend.

/// The seven canonical moods.
///
/// Closed on purpose. The design allows for user-defined moods mapping to a
/// neutral pose; that would be a settings table and a fallback path, and until
/// there is a mood someone actually wants that is not here, it would be
/// machinery serving nobody.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, serde::Serialize, serde::Deserialize,
)]
#[sqlx(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum Mood {
    Content,
    Contemplative,
    Tired,
    Excited,
    Melancholy,
    Chaos,
    /// Chosen deliberately, this means "nothing to report" — which is not the
    /// same as leaving the field unset. See `0003_post_mood.sql`.
    Neutral,
}

impl Mood {
    /// Every mood, in the order the composer offers them and the order ties
    /// break in.
    pub const ALL: [Self; 7] = [
        Self::Content,
        Self::Contemplative,
        Self::Tired,
        Self::Excited,
        Self::Melancholy,
        Self::Chaos,
        Self::Neutral,
    ];

    /// Position in [`Self::ALL`], for counting into a fixed array.
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Content => 0,
            Self::Contemplative => 1,
            Self::Tired => 2,
            Self::Excited => 3,
            Self::Melancholy => 4,
            Self::Chaos => 5,
            Self::Neutral => 6,
        }
    }

    /// The stored form, the wire form, and the displayed form — which are the
    /// same string, so there is one method rather than three that could drift.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Contemplative => "contemplative",
            Self::Tired => "tired",
            Self::Excited => "excited",
            Self::Melancholy => "melancholy",
            Self::Chaos => "chaos",
            Self::Neutral => "neutral",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_is_complete_and_indexes_line_up() {
        // `index` counts into arrays sized by `ALL`; a mood missing from either
        // would be a silent miscount rather than a compile error.
        for (position, mood) in Mood::ALL.into_iter().enumerate() {
            assert_eq!(mood.index(), position, "{mood:?}");
        }
    }

    #[test]
    fn the_stored_name_matches_the_migrations_check_constraint() {
        // These seven strings are written out again in 0003_post_mood.sql, which
        // is a CHECK constraint the database will enforce. A rename here that
        // did not reach that file would fail every write of the renamed value.
        let names: Vec<_> = Mood::ALL.into_iter().map(Mood::as_str).collect();
        assert_eq!(
            names,
            [
                "content",
                "contemplative",
                "tired",
                "excited",
                "melancholy",
                "chaos",
                "neutral"
            ],
        );
    }

    #[test]
    fn json_is_the_lowercase_name() {
        assert_eq!(serde_json::to_string(&Mood::Tired).unwrap(), r#""tired""#);
        assert_eq!(
            serde_json::from_str::<Mood>(r#""melancholy""#).unwrap(),
            Mood::Melancholy,
        );
        assert!(serde_json::from_str::<Mood>(r#""smug""#).is_err());
    }
}
