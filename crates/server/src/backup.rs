//! `youwin-server backup [dir]` — a consistent copy of a live database.
//!
//! `cp` of a WAL database is not a backup: the `.db` file alone can be missing
//! every commit still sitting in the `-wal`, and copying the three files
//! non-atomically can catch them mid-checkpoint. `VACUUM INTO` is SQLite's
//! answer — it reads through a single consistent snapshot and writes a compact,
//! fully self-contained file, with the source open and serving traffic.
//!
//! Doing it here rather than shelling out to `sqlite3` keeps the server's only
//! build and runtime requirement a Rust toolchain, which is the whole reason the
//! deploy is as simple as it is.

use std::{fs, path::Path};

use anyhow::{Context as _, Result, bail};

use crate::{clock::now_millis, db::Db, offsite::Uploader, public::view::time_fmt};

/// Dated backups kept locally before the oldest is removed.
///
/// Retention off-site is deliberately not managed from here: this program has no
/// business deleting objects on a remote it can only append to, and every target
/// worth using has its own lifecycle rules. See [`crate::offsite`].
const KEEP: usize = 30;

pub async fn run(db: &Db, dir: &Path, offsite: &Uploader) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

    let target = dir.join(format!("youwin-{}.db", time_fmt::date(now_millis())));

    // VACUUM INTO refuses to overwrite, which is the right default but wrong for
    // a dated file that a timer may retry. Write beside the target and rename:
    // an interrupted run then leaves a stray `.part` rather than replacing
    // yesterday's good backup with a truncated one.
    let staging = target.with_extension("db.part");
    if staging.exists() {
        fs::remove_file(&staging)
            .with_context(|| format!("clearing {}", staging.display()))?;
    }

    let Some(path) = staging.to_str() else {
        bail!("backup path is not valid UTF-8: {}", staging.display());
    };

    // The write pool: `PRAGMA query_only` on the read pool rejects VACUUM, even
    // though this particular VACUUM does not modify the source.
    sqlx::query("VACUUM INTO ?1")
        .bind(path)
        .execute(&db.write)
        .await
        .with_context(|| format!("vacuuming into {}", staging.display()))?;

    fs::rename(&staging, &target)
        .with_context(|| format!("renaming into place: {}", target.display()))?;

    let bytes = fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    let pruned = prune(dir)?;

    println!(
        "Backed up to {} ({} KiB){}.",
        target.display(),
        bytes / 1024,
        if pruned > 0 {
            format!("; removed {pruned} older than the last {KEEP}")
        } else {
            String::new()
        }
    );

    // Last, and after the rename: what leaves the machine is the finished file,
    // never the `.part`. A failure here fails the whole subcommand — the local
    // snapshot is already safely written, so there is nothing to roll back, and
    // an off-site copy that quietly did not happen is the one outcome this
    // feature exists to make impossible.
    //
    // Re-read from disk rather than kept from the VACUUM: this uploads the file
    // that is actually sitting there, which is the thing a restore would use.
    if offsite.is_enabled() {
        let name = target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("youwin.db");
        let body = fs::read(&target).with_context(|| format!("reading {}", target.display()))?;
        offsite.put(name, "application/octet-stream", body).await?;
    }

    Ok(())
}

/// Removes all but the newest [`KEEP`] dated backups.
///
/// Matches the exact `youwin-YYYY-MM-DD.db` shape and nothing else, so anything
/// else living in the directory — a manual copy, a `.part` from a failed run,
/// somebody's notes — is never a candidate for deletion. Names sort lexically
/// because the date format is zero-padded and big-endian.
fn prune(dir: &Path) -> Result<usize> {
    let mut dated: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("listing {}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| is_dated_backup(name))
        .collect();

    if dated.len() <= KEEP {
        return Ok(0);
    }

    dated.sort();
    let doomed = dated.len() - KEEP;

    for name in dated.iter().take(doomed) {
        fs::remove_file(dir.join(name))
            .with_context(|| format!("removing {name}"))?;
    }

    Ok(doomed)
}

fn is_dated_backup(name: &str) -> bool {
    let Some(date) = name
        .strip_prefix("youwin-")
        .and_then(|rest| rest.strip_suffix(".db"))
    else {
        return false;
    };

    // YYYY-MM-DD, exactly.
    date.len() == 10
        && date.as_bytes().iter().enumerate().all(|(i, b)| {
            if matches!(i, 4 | 7) {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::is_dated_backup;

    #[test]
    fn only_exactly_dated_backups_are_prunable() {
        assert!(is_dated_backup("youwin-2026-08-08.db"));

        // Everything a person might reasonably leave in that directory.
        for safe in [
            "youwin.db",
            "youwin-2026-08-08.db.part",
            "youwin-2026-08-08.db.gz",
            "youwin-before-the-migration.db",
            "youwin-2026-8-8.db",
            "notes.txt",
            "",
        ] {
            assert!(!is_dated_backup(safe), "{safe:?} must never be deleted");
        }
    }
}
