//! Telling Cloudflare to forget the public site after a write.
//!
//! The edge holds server-rendered HTML for `s-maxage=300`, which is what makes
//! the public site cost nothing to serve — and what makes a typo fix take five
//! minutes to appear. This closes that gap when it is configured, and does
//! nothing at all when it is not.
//!
//! **Purges everything, not a list of URLs.** One write can invalidate more
//! pages than it looks like: a reply changes its own permalink, every other
//! permalink in the thread (each renders the whole thread), the feed, the Atom
//! document, each of its tag pages, the tag index, and any cached search results
//! that matched it. Enumerating that correctly is a bug waiting to happen, and
//! the failure mode is silent — a stale page nobody notices. Purging everything
//! cannot be incomplete. The cost is a handful of origin renders and one
//! re-fetch of a 10 kB stylesheet, on a site that is written to a few times a
//! day.

use std::{sync::Arc, time::Duration};

use serde_json::json;

/// Cloudflare's API. Overridable so the request this builds can be pointed at a
/// stub and inspected — the alternative is shipping an HTTP client whose output
/// nobody has ever looked at.
const DEFAULT_API_BASE: &str = "https://api.cloudflare.com/client/v4";

/// A purge must not hold up a reply to the composer. If Cloudflare is slow, the
/// post is already written and the cache will expire on its own.
const TIMEOUT: Duration = Duration::from_secs(10);

pub enum Purger {
    /// No zone or no token configured. Every call is a no-op, which is the
    /// documented default: the TTL alone is a perfectly good answer.
    Disabled,
    Cloudflare {
        client: reqwest::Client,
        endpoint: String,
        token: String,
    },
}

impl Purger {
    /// Builds a purger from configuration, or [`Purger::Disabled`].
    ///
    /// The token needs the `Cache Purge` permission, which the DNS-01 token
    /// Caddy uses does not have and should not be given — that one is scoped to
    /// DNS edits, and widening it to make this work would trade a real boundary
    /// for a small convenience.
    pub fn new(zone_id: Option<&str>, token: Option<&str>, api_base: &str) -> Self {
        let (Some(zone), Some(token)) = (zone_id, token) else {
            return Self::Disabled;
        };

        let base = if api_base.is_empty() {
            DEFAULT_API_BASE
        } else {
            api_base
        };

        match reqwest::Client::builder().timeout(TIMEOUT).build() {
            Ok(client) => Self::Cloudflare {
                client,
                endpoint: format!("{base}/zones/{zone}/purge_cache"),
                token: token.to_owned(),
            },
            Err(error) => {
                // Not fatal. A site that serves slightly stale pages is better
                // than one that refuses to start.
                tracing::error!(%error, "could not build the cache purge client; purging is off");
                Self::Disabled
            }
        }
    }

    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Cloudflare { .. })
    }

    /// Requests a purge in the background.
    ///
    /// Returns immediately and never fails the caller: the write it follows has
    /// already committed, so there is no outcome here that should turn a
    /// successful post into an error. A failure is logged and the TTL takes over.
    pub fn purge_everything(self: &Arc<Self>) {
        let Self::Cloudflare {
            client,
            endpoint,
            token,
        } = self.as_ref()
        else {
            return;
        };

        let request = client
            .post(endpoint)
            .bearer_auth(token)
            .json(&json!({ "purge_everything": true }));

        tokio::spawn(async move {
            match request.send().await {
                Ok(response) if response.status().is_success() => {
                    tracing::info!("purged the edge cache");
                }
                Ok(response) => {
                    let status = response.status();
                    // Cloudflare puts the reason in the body; without it a 403
                    // is indistinguishable from a wrong zone id or a token
                    // missing the purge permission.
                    let body = response.text().await.unwrap_or_default();
                    tracing::warn!(%status, %body, "cache purge refused");
                }
                Err(error) => {
                    tracing::warn!(%error, "cache purge failed");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purging_is_off_unless_both_halves_are_configured() {
        assert!(!Purger::new(None, None, "").is_enabled());
        assert!(!Purger::new(Some("zone"), None, "").is_enabled());
        assert!(!Purger::new(None, Some("token"), "").is_enabled());
        assert!(Purger::new(Some("zone"), Some("token"), "").is_enabled());
    }

    #[test]
    fn the_endpoint_is_built_from_the_zone() {
        let purger = Purger::new(Some("abc123"), Some("t"), "https://stub.test/v4");
        let Purger::Cloudflare { endpoint, .. } = &purger else {
            panic!("expected an enabled purger");
        };
        assert_eq!(endpoint, "https://stub.test/v4/zones/abc123/purge_cache");
    }
}
