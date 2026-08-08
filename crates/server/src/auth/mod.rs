//! Authentication for `write.youwin.dev`.
//!
//! One user, one password, session cookies scoped to this host alone. The public
//! site has no auth surface at all — it never reads a cookie and never issues
//! one.

pub mod middleware;
pub mod password;
pub mod ratelimit;
pub mod session;

use axum::http::HeaderMap;

/// Best-effort client IP, for rate-limit bucketing only.
///
/// Trustworthy *only* because the backend binds loopback and Caddy is the sole
/// possible peer — anything that can reach this port has already been through
/// the proxy. If this ever binds 0.0.0.0, these headers become attacker-supplied
/// and the limiter becomes trivially evadable.
///
/// Cloudflare sets `CF-Connecting-IP`; Caddy appends `X-Forwarded-For`. Falling
/// back to a shared bucket is deliberate: in a world where neither header
/// arrives, everyone throttles together, which fails closed rather than open.
pub fn client_ip(headers: &HeaderMap) -> String {
    if let Some(ip) = headers.get("cf-connecting-ip").and_then(|v| v.to_str().ok()) {
        return ip.to_owned();
    }

    if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        // Left-most entry is the original client; the rest are proxy hops.
        if let Some(first) = forwarded.split(',').next() {
            let trimmed = first.trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }
    }

    "unknown".to_owned()
}

/// Truncated so a hostile `User-Agent` cannot bloat the row.
pub fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|ua| ua.chars().take(256).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn cloudflare_header_wins_over_the_proxy_chain() {
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", HeaderValue::from_static("203.0.113.7"));
        headers.insert("x-forwarded-for", HeaderValue::from_static("10.0.0.1"));

        assert_eq!(client_ip(&headers), "203.0.113.7");
    }

    #[test]
    fn forwarded_for_takes_the_left_most_hop() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.7, 10.0.0.1, 172.16.0.1"),
        );

        assert_eq!(client_ip(&headers), "203.0.113.7");
    }

    #[test]
    fn missing_headers_collapse_to_a_shared_bucket() {
        assert_eq!(client_ip(&HeaderMap::new()), "unknown");

        // An empty value must not produce an empty key that silently matches.
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("  "));
        assert_eq!(client_ip(&headers), "unknown");
    }

    #[test]
    fn user_agent_is_truncated() {
        let mut headers = HeaderMap::new();
        let long = "a".repeat(1000);
        headers.insert(
            axum::http::header::USER_AGENT,
            HeaderValue::from_str(&long).unwrap(),
        );

        assert_eq!(user_agent(&headers).unwrap().len(), 256);
        assert_eq!(user_agent(&HeaderMap::new()), None);
    }
}
