//! Why a `PUT` was not accepted, in a form the sending box can read.
//!
//! [`youwin_server::offsite`] logs the status **and the response body** on a
//! failed upload, with a comment explaining why: without the body, a 403 is
//! indistinguishable from a wrong path, a full quota, or a credential that
//! expired six weeks ago. This is the other end of that decision. Every refusal
//! here carries a sentence saying what was wrong, because the person reading it
//! is looking at `systemctl status youwin-backup` on a different machine at an
//! hour they did not choose.
//!
//! The one exception is [`Refusal::Unauthorized`], which says nothing. Anybody
//! can reach this hostname; only the sender should learn anything from it.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

pub enum Refusal {
    /// Wrong `Authorization`, or none. Terse on purpose.
    Unauthorized,
    /// A request target that is not one of the two names the sender generates.
    /// 400 rather than 404: nothing here is missing, the name is simply not one
    /// this service will write, and "not found" would send somebody looking for
    /// a directory that was never supposed to exist.
    Unacceptable(String),
    /// Bigger than `YOUWIN_OFFSITE_MAX_BYTES`. Caddy's `max_size` normally gets
    /// there first; this is what catches a Caddy block that was loosened without
    /// this being told.
    TooLarge(u64),
    /// The bytes arrived intact and are not a usable backup. The one refusal
    /// that means "your database is bad", not "your request was bad" — hence
    /// 422 rather than 400, so the two are distinguishable in a log a year from
    /// now.
    Corrupt(String),
    /// Something went wrong on this side: a full disk, a permission, a rename.
    /// Not the sender's fault and not something it can fix, but it must still
    /// fail loudly rather than believe a backup landed.
    Failed(anyhow::Error),
}

impl Refusal {
    /// Wraps an error from this side of the wire.
    pub fn failed(error: impl Into<anyhow::Error>) -> Self {
        Self::Failed(error.into())
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized\n".to_owned(),
            ),
            Self::Unacceptable(target) => (
                StatusCode::BAD_REQUEST,
                format!(
                    "{target} is not something this receiver accepts. It answers exactly \
                     `PUT /youwin-YYYY-MM-DD.db` and `PUT /youwin-YYYY-MM-DD.json` at the \
                     root — if this was a nightly backup, check YOUWIN_OFFSITE_URL has no \
                     path component.\n"
                ),
            ),
            Self::TooLarge(max) => (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("larger than the {max} byte limit this receiver accepts\n"),
            ),
            Self::Corrupt(why) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "the upload arrived intact but is not a usable backup: {why}. \
                     Nothing was written; the previous copy is untouched.\n"
                ),
            ),
            Self::Failed(error) => {
                // Logged in full here, summarised on the wire. The sender's
                // journal gets enough to know it is not its problem; the detail
                // stays on the box that can act on it.
                tracing::error!(error = ?error, "could not store an upload");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("the receiver could not store it: {error:#}\n"),
                )
            }
        };

        // Logged at the point of refusal too, so `journalctl -u youwin-offsite`
        // on its own answers "did anything try and get turned away?".
        if status != StatusCode::INTERNAL_SERVER_ERROR {
            tracing::warn!(status = status.as_u16(), reason = %body.trim(), "refused an upload");
        }

        (status, [("content-type", "text/plain; charset=utf-8")], body).into_response()
    }
}
