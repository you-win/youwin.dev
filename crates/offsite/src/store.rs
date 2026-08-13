//! Getting bytes onto the disk in a way that cannot lose the copy already there.
//!
//! The order is the whole module: **write aside, sync, verify, rename, sync the
//! directory, prune.** Every step before the rename is reversible by deleting one
//! `.part` file, and the rename is the single atomic instant where last night's
//! backup becomes tonight's. There is no window in which the file at the final
//! path is anything other than a snapshot that passed [`crate::verify`].
//!
//! That matters more here than in most places, because the sender retries. A
//! timer that fires twice in a day, or a run repeated by hand after a failure,
//! `PUT`s the same name again — so "replace an existing good file" is a routine
//! path, not an exceptional one, and doing it non-atomically would mean the
//! failure mode is *losing a good backup to a bad upload*.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context as _;
use axum::body::Body;
use futures_util::StreamExt as _;
use tokio::io::AsyncWriteExt as _;

use crate::{
    name::{Artifact, Kind},
    refusal::Refusal,
    verify,
};

/// What landed, for the log line and the response.
pub struct Landed {
    pub path: PathBuf,
    pub bytes: u64,
    /// Posts counted in an arriving database; `None` for an export.
    pub posts: Option<i64>,
    pub pruned: usize,
}

pub async fn receive(
    dir: &Path,
    artifact: &Artifact,
    body: Body,
    max_bytes: u64,
    keep: usize,
) -> Result<Landed, Refusal> {
    fs::create_dir_all(dir)
        .with_context(|| format!("creating {}", dir.display()))
        .map_err(Refusal::failed)?;

    let staging = dir.join(artifact.staging_name());
    let target = dir.join(artifact.file_name());

    let bytes = match stream_to(&staging, body, max_bytes).await {
        Ok(bytes) => bytes,
        Err(refusal) => {
            discard(&staging);
            return Err(refusal);
        }
    };

    let posts = match check(&staging, artifact.kind).await {
        Ok(posts) => posts,
        Err(refusal) => {
            // The point of the staging file. Whatever is at `target` is a copy
            // that passed these same checks on some earlier night, and it is
            // still there.
            discard(&staging);
            return Err(refusal);
        }
    };

    fs::rename(&staging, &target)
        .with_context(|| format!("renaming into place: {}", target.display()))
        .map_err(Refusal::failed)?;

    // The file's own contents were synced before the rename; this syncs the
    // directory entry that now points at them. Without it a power loss can leave
    // the rename itself unrecorded — the old name back, or neither name — on a
    // machine whose entire job is surviving the loss of another machine.
    if let Err(error) = sync_dir(dir) {
        // Not fatal. The data is on the platter and the rename has returned; a
        // durability belt-and-braces step failing is worth a line, not a refused
        // backup that is already sitting there correctly.
        tracing::warn!(%error, dir = %dir.display(), "could not fsync the backup directory");
    }

    // Last, and never allowed to fail the upload: the bytes are safely in place
    // by this point, and refusing an arrival that succeeded because tidying up
    // afterwards did not is the wrong trade. It goes to the journal instead.
    let pruned = match prune(dir, keep) {
        Ok(pruned) => pruned,
        Err(error) => {
            tracing::warn!(error = ?error, "could not prune old backups");
            0
        }
    };

    Ok(Landed {
        path: target,
        bytes,
        posts,
        pruned,
    })
}

/// Streams the request body to `path`, refusing anything over `max_bytes`.
///
/// Streamed rather than buffered because this runs on a box with somebody else's
/// service on it. The sending half reads its file into memory on purpose — it is
/// a process that exits immediately afterwards — but a long-lived listener that
/// holds however much Caddy will accept in RAM is a way to take an unrelated
/// service down with an upload.
async fn stream_to(path: &Path, body: Body, max_bytes: u64) -> Result<u64, Refusal> {
    let mut file = tokio::fs::File::create(path)
        .await
        .with_context(|| format!("creating {}", path.display()))
        .map_err(Refusal::failed)?;

    let mut stream = body.into_data_stream();
    let mut written: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .context("reading the request body")
            .map_err(Refusal::failed)?;

        // Checked before the write, not after, so the limit is a ceiling on what
        // touches the disk rather than on what is left there afterwards.
        written += chunk.len() as u64;
        if written > max_bytes {
            return Err(Refusal::TooLarge(max_bytes));
        }

        file.write_all(&chunk)
            .await
            .with_context(|| format!("writing {}", path.display()))
            .map_err(Refusal::failed)?;
    }

    // Before the verify, not just before the rename: the check below should read
    // what the disk has, not what the page cache is still holding.
    file.sync_all()
        .await
        .with_context(|| format!("syncing {}", path.display()))
        .map_err(Refusal::failed)?;

    Ok(written)
}

/// Runs the verifier for this kind against the staging file.
async fn check(path: &Path, kind: Kind) -> Result<Option<i64>, Refusal> {
    match kind {
        Kind::Database => verify::database(path)
            .await
            .map(Some)
            .map_err(|error| Refusal::Corrupt(format!("{error:#}"))),
        Kind::Export => {
            let owned = path.to_owned();
            // An export can be tens of megabytes of JSON, and serde is
            // resolutely synchronous. Off the runtime, so parsing one upload
            // cannot stall the listener answering another.
            tokio::task::spawn_blocking(move || verify::export(&owned))
                .await
                .context("the export verifier panicked")
                .map_err(Refusal::failed)?
                .map(|()| None)
                .map_err(|error| Refusal::Corrupt(format!("{error:#}")))
        }
    }
}

/// Removes a staging file after a failure, complaining rather than caring.
///
/// A leftover `.part` is inert: it does not parse as an [`Artifact`], so it is
/// never served, never counted, and never pruned. It sits there being obvious in
/// `ls`, which is the correct outcome for the debris of a failed night.
fn discard(staging: &Path) {
    if let Err(error) = fs::remove_file(staging)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(%error, path = %staging.display(), "could not remove a staging file");
    }
}

/// Removes all but the newest `keep` of each kind.
///
/// Per-kind, so a missing export can never cost a snapshot, and vice versa —
/// they arrive as a pair but they fail independently.
///
/// The candidate list comes from [`Artifact::parse`], which is the same function
/// that decides what may be *written*. That equivalence is the safety property:
/// this program is structurally incapable of deleting a file it could not have
/// created, so a hand-copied `youwin-before-the-migration.db`, a `.part` from a
/// failed run, or somebody's notes are never candidates. Names sort lexically
/// because the date format is zero-padded and big-endian.
pub fn prune(dir: &Path, keep: usize) -> anyhow::Result<usize> {
    let present: Vec<Artifact> = fs::read_dir(dir)
        .with_context(|| format!("listing {}", dir.display()))?
        .filter_map(Result::ok)
        .filter_map(|entry| Artifact::parse(&entry.file_name().to_string_lossy()))
        .collect();

    let mut removed = 0;

    for kind in Kind::ALL {
        let mut dated: Vec<String> = present
            .iter()
            .filter(|artifact| artifact.kind == kind)
            .map(Artifact::file_name)
            .collect();

        if dated.len() <= keep {
            continue;
        }

        dated.sort();
        let doomed = dated.len() - keep;

        for name in dated.iter().take(doomed) {
            fs::remove_file(dir.join(name)).with_context(|| format!("removing {name}"))?;
            removed += 1;
        }
    }

    Ok(removed)
}

/// fsyncs a directory, so a rename into it survives a power cut.
#[cfg(unix)]
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

/// Windows has no equivalent and is the dev platform only — this service runs on
/// Linux. Silently doing nothing here is honest; the alternative is a `cfg` on
/// the caller and a durability claim that reads as if it applied everywhere.
#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) {
        fs::write(dir.join(name), b"x").expect("write");
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut found: Vec<String> = fs::read_dir(dir)
            .expect("read_dir")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        found.sort();
        found
    }

    #[test]
    fn each_kind_is_kept_to_its_own_limit() {
        let dir = tempfile::TempDir::new().expect("tempdir");

        // Five days of pairs, keeping two.
        for day in 1..=5 {
            touch(dir.path(), &format!("youwin-2026-08-0{day}.db"));
            touch(dir.path(), &format!("youwin-2026-08-0{day}.json"));
        }

        assert_eq!(prune(dir.path(), 2).expect("prune"), 6);
        assert_eq!(
            names(dir.path()),
            [
                "youwin-2026-08-04.db",
                "youwin-2026-08-04.json",
                "youwin-2026-08-05.db",
                "youwin-2026-08-05.json",
            ],
        );
    }

    #[test]
    fn an_uneven_pair_does_not_drag_the_other_kind_down() {
        let dir = tempfile::TempDir::new().expect("tempdir");

        // Four snapshots, one export — what a week of exports failing verification
        // while snapshots kept landing actually leaves behind.
        for day in 1..=4 {
            touch(dir.path(), &format!("youwin-2026-08-0{day}.db"));
        }
        touch(dir.path(), "youwin-2026-08-01.json");

        assert_eq!(prune(dir.path(), 2).expect("prune"), 2);
        assert!(
            names(dir.path()).contains(&"youwin-2026-08-01.json".to_owned()),
            "the only export must survive a snapshot sweep",
        );
    }

    #[test]
    fn nothing_it_could_not_have_written_is_ever_deleted() {
        let dir = tempfile::TempDir::new().expect("tempdir");

        for day in 1..=3 {
            touch(dir.path(), &format!("youwin-2026-08-0{day}.db"));
        }

        // Everything a person might reasonably leave in a backup directory, plus
        // the debris this program itself leaves after a refused upload.
        let untouchable = [
            "youwin-before-the-migration.db",
            "youwin-2026-08-09.db.part",
            "youwin-2026-08-01.db.gz",
            "restore-notes.txt",
            "posts.json",
        ];
        for name in untouchable {
            touch(dir.path(), name);
        }

        assert_eq!(prune(dir.path(), 1).expect("prune"), 2);

        let left = names(dir.path());
        for name in untouchable {
            assert!(left.contains(&name.to_owned()), "{name} must never be deleted");
        }
    }

    #[test]
    fn keeping_more_than_there_are_removes_nothing() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        touch(dir.path(), "youwin-2026-08-01.db");

        assert_eq!(prune(dir.path(), 90).expect("prune"), 0);
        assert_eq!(names(dir.path()), ["youwin-2026-08-01.db"]);
    }
}
