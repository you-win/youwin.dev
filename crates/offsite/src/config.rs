//! Everything the process needs, resolved from the environment once at startup.
//!
//! Same reasoning as the server's [`youwin_server::config`]: a missing or
//! malformed value is a startup failure systemd surfaces immediately, not a 500
//! discovered the night it mattered. The variables share the sender's
//! `YOUWIN_OFFSITE_` prefix deliberately — the two halves are one feature, and
//! `YOUWIN_OFFSITE_AUTH` in particular holds *the same string* on both boxes.

use std::{env, net::SocketAddr, path::PathBuf, str::FromStr};

use anyhow::{Context as _, Result, bail};

/// 512 MiB, matching the `request_body max_size` in the Caddy block.
///
/// The two are independent limits on the same thing, which is worth saying out
/// loud: Caddy's rejects the request before a byte reaches this process, so it
/// is the one that normally fires. This one exists because a Caddyfile can be
/// edited by somebody who does not know this program is behind it.
const DEFAULT_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Roughly three months of nightly pairs.
///
/// Longer than the 30 the sender keeps locally, because the two retentions are
/// for different failures. Local snapshots cover a bad write you notice this
/// week. This one covers "the box is gone", and also "the corruption started
/// before the oldest copy I still have" — which is the failure that eats a short
/// retention whole.
const DEFAULT_KEEP: usize = 90;

#[derive(Debug, Clone)]
pub struct Config {
    /// Loopback, always, in every deployment that should exist. Caddy is the
    /// only thing that should be able to reach this.
    pub bind: SocketAddr,

    /// Where arrivals land. One flat directory of dated files — no per-day
    /// nesting, because `ls -lt` answering "did last night work?" at a glance is
    /// the only status interface this service has.
    pub dir: PathBuf,

    /// The complete `Authorization` header value expected from the sender —
    /// `Bearer …`, `Basic …`, whatever was configured over there. Compared
    /// whole, so there is no scheme setting to disagree about.
    ///
    /// Not optional, unlike the sender's. The sender may legitimately have no
    /// credential when its target authenticates through the URL; this listens on
    /// a public hostname, where "no credential" means a public drop box.
    pub auth: String,

    pub keep: usize,
    pub max_bytes: u64,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let Some(auth) = optional_env("YOUWIN_OFFSITE_AUTH") else {
            bail!(
                "YOUWIN_OFFSITE_AUTH is not set. It must be the same complete Authorization \
                 header value the sending box has in its own YOUWIN_OFFSITE_AUTH — this \
                 refuses to start without it rather than accept uploads from anybody."
            );
        };

        let keep = parse_env::<usize>("YOUWIN_OFFSITE_KEEP", &DEFAULT_KEEP.to_string())?;
        if keep == 0 {
            bail!("YOUWIN_OFFSITE_KEEP is 0, which would delete each backup as it arrived");
        }

        Ok(Self {
            bind: parse_env("YOUWIN_OFFSITE_BIND", "127.0.0.1:8080")?,
            // Relative by default so `cargo run` works in a checkout without
            // writing to a system path; the unit file sets the real one.
            dir: env_or("YOUWIN_OFFSITE_DIR", "backups").into(),
            auth,
            keep,
            max_bytes: parse_env("YOUWIN_OFFSITE_MAX_BYTES", &DEFAULT_MAX_BYTES.to_string())?,
        })
    }
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}

/// An unset variable and one set to the empty string mean the same thing —
/// systemd `Environment=` lines get commented out by blanking them as often as
/// by deleting them.
fn optional_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.is_empty())
}

fn parse_env<T>(key: &str, default: &str) -> Result<T>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let raw = env_or(key, default);
    raw.parse()
        .with_context(|| format!("{key} is not a valid value: {raw:?}"))
}
