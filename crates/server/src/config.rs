use std::{env, net::SocketAddr, path::PathBuf, str::FromStr};

use anyhow::{Context as _, Result};

/// Everything the process needs, resolved from the environment once at startup.
///
/// Reading configuration exactly once — rather than calling `env::var` from
/// handlers — means a missing or malformed value is a startup failure that
/// systemd will surface immediately, not a 500 discovered weeks later.
#[derive(Debug, Clone)]
// Several fields are read only from M2 (auth) and M3 (the authoring app) onward.
// They are resolved now so that a bad value fails at startup from the first
// deploy rather than the first login.
#[allow(dead_code)]
pub struct Config {
    pub database_path: PathBuf,

    /// Vite's build output for the public site. Read once at startup for the
    /// asset manifest; Caddy serves the files themselves.
    pub public_dist: PathBuf,

    /// `youwin.dev` — the public archive. Read pool only.
    pub public_bind: SocketAddr,
    /// `write.youwin.dev` — the authoring API. Read and write pools.
    pub write_bind: SocketAddr,

    /// Absolute origins, used for canonical URLs, Atom links, and the `Origin`
    /// check on state-changing requests.
    pub public_origin: String,
    pub write_origin: String,

    /// `argon2id` PHC string. Optional until M2 introduces auth; once the login
    /// route exists this becomes a hard startup requirement.
    pub password_hash: Option<String>,

    /// Drives both the `Secure` attribute and the `__Host-` cookie prefix. False
    /// in dev so `http://localhost` works without special-casing browsers.
    pub cookie_secure: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            database_path: env_or("YOUWIN_DB", "youwin.db").into(),
            public_dist: env_or("YOUWIN_PUBLIC_DIST", "web/dist/public").into(),
            public_bind: parse_env("YOUWIN_PUBLIC_BIND", "127.0.0.1:8080")?,
            write_bind: parse_env("YOUWIN_WRITE_BIND", "127.0.0.1:8081")?,
            public_origin: env_or("YOUWIN_PUBLIC_ORIGIN", "http://localhost:8080"),
            write_origin: env_or("YOUWIN_WRITE_ORIGIN", "http://localhost:5173"),
            password_hash: env::var("YOUWIN_PASSWORD_HASH").ok().filter(|s| !s.is_empty()),
            cookie_secure: parse_env("YOUWIN_COOKIE_SECURE", "false")?,
        })
    }
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
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
