//! `youwin-offsite` — the far end of the nightly `PUT`.
//!
//! [`youwin_server::offsite`] sends two files a night to a URL it is given, and
//! is deliberately incurious about what answers: a Storage Box, rsync.net, an
//! nginx with `dav_methods`. All of those work. This is what runs instead when
//! you would rather the far end were a machine you own, and it earns that choice
//! by doing the one thing a dumb file server structurally cannot.
//!
//! **It opens the database before it accepts it.** A WebDAV server stores 40MB
//! and returns 201. It cannot tell a `VACUUM INTO` snapshot from 40MB of zeroes,
//! and neither can the sender — a successful `PUT` is the only signal it has. So
//! every arriving `.db` is opened and put through `PRAGMA integrity_check`, and
//! every arriving `.json` is parsed, *before* it is renamed into place. A file
//! that fails is deleted and the request is refused, which turns the sending
//! box's unit red. The entire value of this program is that the night a backup
//! goes bad is the night you find out, rather than the day you need it.
//!
//! **It can only write names it could have generated.** The path segment from
//! the request is parsed into a date and a kind and then *thrown away*; what
//! lands on disk is rebuilt from those parts by [`name::Artifact::file_name`].
//! Traversal, absolute paths, `..`, NUL bytes, and Unicode lookalikes are not
//! defended against so much as unrepresentable — there is no code path from an
//! attacker-controlled string to a filesystem path. The same parser decides what
//! [`store::prune`] is allowed to delete, so this program can only ever remove a
//! file it could have written.
//!
//! **Caddy in front does the parts it is better at.** TLS, the 512MB body cap,
//! and refusing every method except `PUT` — `handle { abort }` — are all in the
//! site block, not here. What Caddy cannot do is authenticate, so
//! [`http`] does, comparing the whole `Authorization` header against
//! `YOUWIN_OFFSITE_AUTH`: the same complete-header-value convention the sender
//! uses, so both boxes hold the same string under the same name and there is
//! nothing to translate. A missing value is a startup failure — this listens on
//! a hostname the world can reach, and an unauthenticated one is a public
//! drop box.
//!
//! **Nothing here is reachable except by `PUT`.** There is no health endpoint,
//! because the Caddy block would abort it; the honest status check is
//! `ls -l` on the directory and `journalctl`, and the deploy README says so
//! rather than shipping a route that only answers in dev.

pub mod config;
pub mod http;
pub mod name;
pub mod refusal;
pub mod store;
pub mod verify;
