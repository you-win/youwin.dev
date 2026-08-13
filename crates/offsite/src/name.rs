//! The only names this service will ever write, and the only ones it will ever
//! delete.
//!
//! [`Artifact::parse`] is the security boundary of the whole program. It takes
//! the path segment out of the request and either understands it completely — a
//! kind and a calendar date — or rejects it. Nothing in between, and no
//! sanitizing: a name is never *cleaned up* into something safe, because that is
//! the pattern where `....//` and `%2e%2e%2f` and a NUL byte in the middle each
//! need their own defence.
//!
//! What reaches the filesystem is [`Artifact::file_name`], rebuilt from the
//! parsed parts. It is a `format!` over a validated date and a fixed extension,
//! so there is no path from the request's bytes to the path on disk at all.

/// The two artifacts a nightly run sends.
///
/// Deliberately closed. A third kind is a change here and in the sender
/// together, which is the point — an unexpected name arriving in the night is
/// something to refuse and log, not something to store because it looked
/// harmless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `youwin-YYYY-MM-DD.db` — the `VACUUM INTO` snapshot, what a restore uses.
    Database,
    /// `youwin-YYYY-MM-DD.json` — the dated `posts.json`, complete enough to
    /// rebuild the database and readable with no SQLite and no toolchain.
    Export,
}

impl Kind {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Database => "db",
            Self::Export => "json",
        }
    }

    /// For the log line, so `journalctl` reads as prose rather than extensions.
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Database => "database snapshot",
            Self::Export => "export",
        }
    }

    /// Both kinds, for the callers that have to sweep each one separately —
    /// retention is per-kind, so losing a `.json` must never cost a `.db`.
    pub const ALL: [Self; 2] = [Self::Database, Self::Export];
}

/// A name this service accepts, taken apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub kind: Kind,
    /// `YYYY-MM-DD`, and nothing else can be in here — see [`is_plausible_date`].
    date: String,
}

impl Artifact {
    /// The whole of the untrusted-input handling in this program.
    ///
    /// Returns `None` for anything that is not exactly one of the two names the
    /// sender generates. That includes names that are merely *close*:
    /// `youwin-2026-8-9.db` (unpadded), `youwin-2026-08-09.db.part` (our own
    /// staging suffix), `youwin-2026-08-09.DB` (case), and every spelling of a
    /// directory traversal, which cannot survive a `-` at index 4.
    pub fn parse(name: &str) -> Option<Self> {
        let rest = name.strip_prefix("youwin-")?;

        // Longest extension first would matter if one were a suffix of the
        // other; they are not, but the order is fixed here rather than left to
        // depend on that staying true.
        let (date, kind) = if let Some(date) = rest.strip_suffix(".db") {
            (date, Kind::Database)
        } else if let Some(date) = rest.strip_suffix(".json") {
            (date, Kind::Export)
        } else {
            return None;
        };

        if !is_plausible_date(date) {
            return None;
        }

        Some(Self {
            kind,
            date: date.to_owned(),
        })
    }

    /// What actually gets written. Built from the parsed parts, never from the
    /// request — see the module docs.
    pub fn file_name(&self) -> String {
        format!("youwin-{}.{}", self.date, self.kind.extension())
    }

    /// Where the bytes go until they have been verified.
    ///
    /// `.part` matches what the sender's own `backup` uses for the same reason
    /// and, more usefully, does not parse as an [`Artifact`] — so an interrupted
    /// upload leaves a file that [`crate::store::prune`] is structurally
    /// incapable of deleting and `ls` makes obvious.
    pub fn staging_name(&self) -> String {
        format!("{}.part", self.file_name())
    }

    pub fn date(&self) -> &str {
        &self.date
    }
}

/// `YYYY-MM-DD`, zero-padded, with a month and day that could exist.
///
/// The range check does not make anything safer — the shape check already did
/// that — but `youwin-2026-99-99.db` is not a file the sender can produce, and a
/// backup directory whose names sort lexically only stays sorted while they are
/// all real dates.
fn is_plausible_date(date: &str) -> bool {
    let bytes = date.as_bytes();

    if bytes.len() != 10 {
        return false;
    }

    let shaped = bytes.iter().enumerate().all(|(i, byte)| {
        if matches!(i, 4 | 7) {
            *byte == b'-'
        } else {
            byte.is_ascii_digit()
        }
    });
    if !shaped {
        return false;
    }

    // Indexing is safe: every byte is ASCII, checked above.
    let month = date[5..7].parse::<u8>().unwrap_or(0);
    let day = date[8..10].parse::<u8>().unwrap_or(0);

    (1..=12).contains(&month) && (1..=31).contains(&day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_names_the_sender_generates_are_accepted() {
        let db = Artifact::parse("youwin-2026-08-09.db").expect("a dated snapshot");
        assert_eq!(db.kind, Kind::Database);
        assert_eq!(db.date(), "2026-08-09");
        assert_eq!(db.file_name(), "youwin-2026-08-09.db");

        let json = Artifact::parse("youwin-2026-08-09.json").expect("a dated export");
        assert_eq!(json.kind, Kind::Export);
        assert_eq!(json.file_name(), "youwin-2026-08-09.json");
    }

    #[test]
    fn a_parsed_name_round_trips_exactly() {
        // The property the whole design rests on: what lands on disk is what
        // arrived, or nothing arrived at all. If these two ever disagreed, the
        // rebuild-from-parts defence would be silently writing somewhere else.
        for name in ["youwin-2026-08-09.db", "youwin-1999-12-31.json"] {
            assert_eq!(Artifact::parse(name).expect("parses").file_name(), name);
        }
    }

    #[test]
    fn nothing_that_could_escape_the_directory_parses() {
        for hostile in [
            "../../../etc/passwd",
            "youwin-../../../etc/passwd.db",
            "youwin-2026-08-09.db/../../etc/cron.d/x",
            "/etc/youwin/secrets.env",
            "youwin-2026-08-09.db\0.txt",
            "youwin-....-..-...db",
            "C:\\Windows\\System32\\config\\SAM",
            "youwin-%2e%2e%2f.db",
            // A real date, but reaching sideways out of the backup directory.
            "../youwin-2026-08-09.db",
        ] {
            assert!(
                Artifact::parse(hostile).is_none(),
                "{hostile:?} must not parse",
            );
        }
    }

    #[test]
    fn names_that_are_merely_close_are_refused() {
        for near_miss in [
            // Our own staging file, which must never be mistaken for an arrival.
            "youwin-2026-08-09.db.part",
            // The sender pads; anything that does not is not from the sender.
            "youwin-2026-8-9.db",
            "youwin-26-08-09.db",
            // Extensions this service has no verifier for.
            "youwin-2026-08-09.db.gz",
            "youwin-2026-08-09.tar",
            "youwin-2026-08-09.DB",
            // The local-only export tree, which deliberately never leaves.
            "posts.json",
            "youwin.db",
            // Impossible dates.
            "youwin-2026-13-01.db",
            "youwin-2026-00-09.db",
            "youwin-2026-08-32.db",
            "youwin-2026-08-00.json",
            "",
            "youwin-.db",
        ] {
            assert!(
                Artifact::parse(near_miss).is_none(),
                "{near_miss:?} must not parse",
            );
        }
    }

    #[test]
    fn a_leap_day_is_not_rejected_by_the_range_check() {
        // The check is a range, not a calendar, and it must stay that way: the
        // sender stamps the date, and this program having its own opinion about
        // which dates exist is a way to refuse a real backup.
        assert!(Artifact::parse("youwin-2028-02-29.db").is_some());
        // Also true of a date that is not a real one but is in range. Storing it
        // costs nothing; refusing a genuine upload over a calendar disagreement
        // would cost a night's backup.
        assert!(Artifact::parse("youwin-2026-02-31.db").is_some());
    }
}
