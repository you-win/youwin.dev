//! Getting a copy of the archive off the machine it lives on.
//!
//! `backup` writes 30 dated SQLite snapshots and `export` writes a JSON and
//! markdown tree — both to a directory on the same disk as the database they
//! protect. That covers a bad write, a bad migration, and a deletion regretted a
//! week later. It does not cover the disk, the machine, or the account, and the
//! failure it does not cover is the one that takes everything at once.
//!
//! So: one HTTPS `PUT` per artifact, to a URL you configure, and nothing at all
//! when you have not. It is off by default and off is a supported way to run.
//!
//! **A plain PUT, not an SDK.** The target is whatever answers `PUT {base}/{name}`
//! with an optional `Authorization` header — a Storage Box, rsync.net, Nextcloud,
//! an S3 gateway, a WebDAV server, an nginx with `dav_methods`. That covers every
//! cheap off-site option worth having without a provider SDK, a signing
//! algorithm, or a credential format this program has to understand. `reqwest`
//! is already a dependency for the cache purge, so the whole feature costs no
//! new crates.
//!
//! **Failures are loud.** Unlike [`crate::cache`], which is spawned and ignored
//! because the write it follows has already committed, an upload that did not
//! happen is exactly the thing this exists to prevent. It propagates, the
//! subcommand exits non-zero, and the systemd unit goes to `failed` where a
//! nightly timer's failure is visible. A backup you believe in and do not have
//! is worse than no backup.

use std::time::Duration;

use anyhow::{Context as _, Result, bail};

/// Generous, because this uploads a whole database over somebody's home
/// connection at 3am with nothing waiting on it. The purger's ten seconds are
/// sized for a request a person is waiting behind; this is the opposite case.
const TIMEOUT: Duration = Duration::from_secs(300);

pub enum Uploader {
    /// No URL configured. Every call is a no-op and `backup`/`export` behave
    /// exactly as they did before this module existed.
    Disabled,
    Http {
        client: reqwest::Client,
        /// Without a trailing slash — [`Uploader::put`] adds exactly one.
        base: String,
        /// A complete `Authorization` header value, or `None` for a target that
        /// authenticates through the URL itself (a presigned or capability URL).
        ///
        /// The whole header rather than a token plus a scheme setting: `Bearer`,
        /// `Basic`, and the provider-specific ones all differ only in a prefix,
        /// and a scheme knob would be a second thing to configure that can only
        /// ever be wrong.
        auth: Option<String>,
    },
}

impl Uploader {
    pub fn new(url: Option<&str>, auth: Option<&str>) -> Self {
        let Some(url) = url else {
            return Self::Disabled;
        };

        // Not fatal, and not silently downgraded either. Shipping the entire
        // archive in clear text is a decision somebody should have made on
        // purpose, and the only case where it is reasonable — a stub on
        // loopback, which is how this module is tested — is obvious in the log.
        if !url.starts_with("https://") {
            tracing::warn!(
                %url,
                "the off-site backup URL is not https; the archive will be uploaded in clear text",
            );
        }

        match reqwest::Client::builder().timeout(TIMEOUT).build() {
            Ok(client) => Self::Http {
                client,
                base: url.trim_end_matches('/').to_owned(),
                auth: auth.map(str::to_owned),
            },
            Err(error) => {
                // Fatal in spirit, but this is constructed at the top of a
                // subcommand and the first `put` will fail loudly anyway.
                tracing::error!(%error, "could not build the off-site client; uploads are off");
                Self::Disabled
            }
        }
    }

    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Http { .. })
    }

    /// Uploads one artifact, or does nothing when no URL is configured.
    ///
    /// `name` is the last path segment, and is never taken from user input —
    /// both callers build it from a date and a fixed stem.
    ///
    /// The body is read into memory rather than streamed. A personal microblog's
    /// database is measured in megabytes; streaming would want `reqwest`'s
    /// `stream` feature and a `File` handle threaded through both callers, to
    /// save a buffer on a process that exits immediately afterwards.
    pub async fn put(&self, name: &str, content_type: &str, body: Vec<u8>) -> Result<()> {
        let Self::Http { client, base, auth } = self else {
            return Ok(());
        };

        let url = format!("{base}/{name}");
        let bytes = body.len();

        let mut request = client
            .put(&url)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body);

        if let Some(auth) = auth {
            request = request.header(reqwest::header::AUTHORIZATION, auth);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("uploading {name} to {base}"))?;

        let status = response.status();
        if !status.is_success() {
            // The body carries the reason on every one of these targets, and
            // without it a 403 is indistinguishable from a wrong path, a full
            // quota, or a credential that expired six weeks ago.
            let detail = response.text().await.unwrap_or_default();
            bail!("uploading {name} to {base} failed: {status} {detail}");
        }

        tracing::info!(name, bytes, "uploaded off-site");
        println!("Uploaded {name} ({} KiB) to {base}.", bytes / 1024);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploads_are_off_until_a_url_is_configured() {
        assert!(!Uploader::new(None, None).is_enabled());
        // Auth alone is not enough to guess a destination from.
        assert!(!Uploader::new(None, Some("Bearer t")).is_enabled());
        assert!(Uploader::new(Some("https://example.test/youwin"), None).is_enabled());
    }

    #[test]
    fn a_trailing_slash_on_the_base_does_not_become_a_double_slash() {
        // Both spellings are what somebody would paste out of a provider's UI,
        // and `//` is a different path on enough servers to matter.
        for base in ["https://example.test/youwin", "https://example.test/youwin/"] {
            let Uploader::Http { base, .. } = Uploader::new(Some(base), None) else {
                panic!("expected an enabled uploader");
            };
            assert_eq!(base, "https://example.test/youwin");
        }
    }

    #[tokio::test]
    async fn a_disabled_uploader_accepts_everything_and_does_nothing() {
        // `backup` and `export` call this unconditionally; being disabled must
        // not be an error path they have to branch around.
        Uploader::Disabled
            .put("youwin-2026-08-09.db", "application/octet-stream", vec![1, 2, 3])
            .await
            .expect("a no-op cannot fail");
    }
}
