//! Session tokens and the cookie that carries them.

use axum_extra::extract::cookie::{Cookie, SameSite};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng as _;
use sha2::{Digest as _, Sha256};

/// 90 days. Long, because the phone is the primary client and being logged out
/// of your own blog is pure friction.
pub const TTL_DAYS: i64 = 90;

/// Only refresh `expires_at` when the session is more than a day stale. Every
/// request refreshing it would be a write on every read.
pub const REFRESH_AFTER_MILLIS: i64 = 24 * 60 * 60 * 1000;

pub const TTL_MILLIS: i64 = TTL_DAYS * 24 * 60 * 60 * 1000;

/// The `__Host-` prefix forbids a `Domain` attribute and requires `Secure` plus
/// `Path=/`, which binds the cookie to exactly `write.youwin.dev`. It therefore
/// *cannot* be sent to youwin.dev — which is what makes the public site
/// anonymous by construction and safe to cache at the edge.
const SECURE_NAME: &str = "__Host-yw_session";

/// Dev only. `__Host-` requires Secure, and Secure over plain http://localhost
/// is inconsistent across browsers — not worth debugging for a dev convenience.
const INSECURE_NAME: &str = "yw_session";

pub fn cookie_name(secure: bool) -> &'static str {
    if secure { SECURE_NAME } else { INSECURE_NAME }
}

/// A freshly minted session token.
///
/// `value` goes to the browser and is never stored; `hash` is stored and never
/// leaves the process. A database leak — or a stray backup on a laptop — then
/// hands over no usable sessions.
pub struct Token {
    pub value: String,
    pub hash: Vec<u8>,
}

pub fn new_token() -> Token {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let value = URL_SAFE_NO_PAD.encode(bytes);
    let hash = hash_token(&value);
    Token { value, hash }
}

pub fn hash_token(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}

pub fn build_cookie(value: String, secure: bool) -> Cookie<'static> {
    Cookie::build((cookie_name(secure), value))
        .http_only(true)
        .secure(secure)
        // Lax is what makes a CSRF token unnecessary: it withholds the cookie
        // from cross-site POST/PATCH/DELETE, which is every mutating route here.
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(time::Duration::days(TTL_DAYS))
        .build()
}

/// An already-expired cookie with the same name, path, and flags, which is the
/// only way to make a browser drop the original.
pub fn clearing_cookie(secure: bool) -> Cookie<'static> {
    Cookie::build((cookie_name(secure), ""))
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(time::Duration::ZERO)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unguessable_and_stored_only_as_hashes() {
        let a = new_token();
        let b = new_token();

        assert_ne!(a.value, b.value);
        assert_eq!(a.hash.len(), 32, "SHA-256");
        assert_eq!(a.hash, hash_token(&a.value));
        assert_ne!(a.hash, b.hash);

        // 32 random bytes, base64url — the cookie value must survive a header
        // unescaped.
        assert_eq!(a.value.len(), 43);
        assert!(
            a.value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        // The stored form must not contain the token.
        assert!(!a.hash.starts_with(a.value.as_bytes()));
    }

    #[test]
    fn the_secure_cookie_carries_every_attribute_that_matters() {
        let cookie = build_cookie("tok".to_owned(), true);

        assert_eq!(cookie.name(), "__Host-yw_session");
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.secure(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.path(), Some("/"));
        // __Host- forbids Domain; setting one would silently void the prefix.
        assert_eq!(cookie.domain(), None);
    }

    #[test]
    fn the_dev_cookie_drops_the_host_prefix_with_secure() {
        let cookie = build_cookie("tok".to_owned(), false);
        assert_eq!(cookie.name(), "yw_session");
        assert_eq!(cookie.secure(), Some(false));
    }

    #[test]
    fn the_clearing_cookie_matches_the_original_so_browsers_drop_it() {
        for secure in [true, false] {
            let live = build_cookie("tok".to_owned(), secure);
            let dead = clearing_cookie(secure);

            assert_eq!(dead.name(), live.name());
            assert_eq!(dead.path(), live.path());
            assert_eq!(dead.secure(), live.secure());
            assert_eq!(dead.value(), "");
            assert_eq!(dead.max_age(), Some(time::Duration::ZERO));
        }
    }
}
