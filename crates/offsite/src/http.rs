//! The one route, and the credential check Caddy cannot do.
//!
//! The site block in front of this refuses every method but `PUT` and caps the
//! body at 512MB, so neither is re-implemented here. What is left is the part a
//! reverse proxy has no opinion about: who is allowed to write, and under what
//! name. Both are answered before a single byte of the body is read.

use std::{path::PathBuf, sync::Arc};

use axum::{
    Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{IntoResponse as _, Response},
    routing::put,
};
use sha2::{Digest as _, Sha256};

use crate::{config::Config, name::Artifact, refusal::Refusal, store};

/// Everything the handler needs, shared across requests.
pub struct Receiver {
    dir: PathBuf,
    auth: String,
    keep: usize,
    max_bytes: u64,
}

impl Receiver {
    pub fn new(cfg: &Config) -> Self {
        Self {
            dir: cfg.dir.clone(),
            auth: cfg.auth.clone(),
            keep: cfg.keep,
            max_bytes: cfg.max_bytes,
        }
    }

    /// Compares the whole `Authorization` header against the configured value.
    ///
    /// Whole, because that is the convention the sending half established: it
    /// holds a complete header value rather than a token plus a scheme setting,
    /// so `Bearer`, `Basic` and anything else differ only in a prefix and there
    /// is nothing on either side that has to understand which. Both boxes hold
    /// the same string under the same variable name.
    ///
    /// The comparison is on SHA-256 digests rather than the values themselves.
    /// `==` on the digests is not constant-time, but what its timing leaks is
    /// how many leading bytes of a *digest* an attacker matched, and there is no
    /// route from that back to the credential — which is the same reasoning that
    /// makes a byte-wise comparison of the raw secret unacceptable.
    fn authorized(&self, headers: &HeaderMap) -> bool {
        let Some(offered) = headers.get(header::AUTHORIZATION) else {
            return false;
        };

        Sha256::digest(offered.as_bytes()) == Sha256::digest(self.auth.as_bytes())
    }
}

pub fn router(receiver: Arc<Receiver>) -> Router {
    Router::new()
        .route("/{name}", put(receive))
        // axum caps request bodies at 2MB by default, which is a sensible
        // default for an API and exactly wrong for this. The real ceilings are
        // Caddy's `max_size` and YOUWIN_OFFSITE_MAX_BYTES, both of which reject
        // rather than truncate; `store` enforces the latter as it streams.
        .layer(DefaultBodyLimit::disable())
        .fallback(unexpected)
        .with_state(receiver)
}

async fn receive(
    State(receiver): State<Arc<Receiver>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, Refusal> {
    // First, and before the name is even looked at. An unauthenticated caller
    // learns nothing from this service — not which names it wants, not whether
    // one already exists.
    if !receiver.authorized(&headers) {
        return Err(Refusal::Unauthorized);
    }

    let Some(artifact) = Artifact::parse(&name) else {
        return Err(Refusal::Unacceptable(format!("{name:?}")));
    };

    let landed = store::receive(
        &receiver.dir,
        &artifact,
        body,
        receiver.max_bytes,
        receiver.keep,
    )
    .await?;

    let kibibytes = landed.bytes / 1024;

    tracing::info!(
        name = %artifact.file_name(),
        kind = artifact.kind.describe(),
        bytes = landed.bytes,
        posts = landed.posts,
        pruned = landed.pruned,
        "stored an off-site backup",
    );

    // 201, and a body a person would want to see from `curl -T`. The sender
    // treats any 2xx as success and does not read this; it is for the day
    // somebody is testing the credential by hand.
    let mut summary = format!(
        "Stored {} ({kibibytes} KiB), verified.\n",
        landed.path.display(),
    );
    if let Some(posts) = landed.posts {
        summary.push_str(&format!("{posts} posts.\n"));
    }
    if landed.pruned > 0 {
        summary.push_str(&format!(
            "Removed {} beyond the {} kept.\n",
            landed.pruned, receiver.keep,
        ));
    }

    Ok((
        StatusCode::CREATED,
        [("content-type", "text/plain; charset=utf-8")],
        summary,
    )
        .into_response())
}

/// Anything that is not `PUT /<name>`.
///
/// Unreachable in production — the Caddy block aborts every other method, and
/// nothing but this service listens on the port. It exists for `cargo run` and,
/// more usefully, for the day somebody sets `YOUWIN_OFFSITE_URL` with a path on
/// the end and needs the 400 to say so rather than a bare axum 404 with no body.
async fn unexpected(method: Method, uri: Uri) -> Refusal {
    Refusal::Unacceptable(format!("`{method} {}`", uri.path()))
}
