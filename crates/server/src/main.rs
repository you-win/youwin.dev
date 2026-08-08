use std::{
    future::IntoFuture,
    io::{IsTerminal as _, Read as _},
};

use anyhow::{Context as _, Result, bail};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;
use youwin_server::{
    auth::password, backup, clock::now_millis, config, db, export, public, seed, write,
};

/// Identifies the build, for the smoke test the server runs before activating a
/// release. CI sets this to the release name; a local build says `dev`.
const BUILD: &str = match option_env!("YOUWIN_BUILD") {
    Some(build) => build,
    None => "dev",
};

#[tokio::main]
async fn main() -> Result<()> {
    // Answered before anything else is touched — no logging, no environment, no
    // database. `activate-youwin` runs this against a freshly uploaded binary to
    // prove it loads on this machine, so it has to work on a box where nothing
    // is configured yet, and it has to be the cheapest thing the program can do.
    //
    // The failure it exists to catch is a glibc mismatch between the CI runner
    // and the server, which otherwise surfaces as a service that will not start
    // *after* the old one has already been stopped.
    if matches!(
        std::env::args().nth(1).as_deref(),
        Some("version" | "--version" | "-V")
    ) {
        println!("youwin-server {} ({BUILD})", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "youwin_server=info,tower_http=warn".into()),
        )
        .init();

    let cfg = config::Config::from_env()?;

    // Hand-rolled rather than a CLI crate: a handful of subcommands with at most
    // one positional argument between them, and a parser for that would be more
    // code than the features.
    match std::env::args().nth(1).as_deref() {
        None => serve(cfg).await,
        Some("hash-password") => hash_password(),
        Some("seed") => with_db(cfg, |db| Box::pin(async move { seed::run(db).await })).await,
        Some("export") => {
            // Defaults to a directory beside the database rather than the
            // working directory: this is most often run from a systemd timer,
            // where the working directory is not something you chose.
            let dir = std::env::args().nth(2).map_or_else(
                || cfg.database_path.with_file_name("export"),
                std::path::PathBuf::from,
            );
            with_db(cfg, move |db| {
                Box::pin(async move { export::run(db, &dir).await })
            })
            .await
        }
        Some("backup") => {
            let dir = std::env::args().nth(2).map_or_else(
                || cfg.database_path.with_file_name("backups"),
                std::path::PathBuf::from,
            );
            with_db(cfg, move |db| {
                Box::pin(async move { backup::run(db, &dir).await })
            })
            .await
        }
        Some("rerender") => {
            with_db(cfg, |db| {
                Box::pin(async move {
                    let result = db::posts::rerender_all(&db.write).await?;
                    println!(
                        "Re-rendered {} posts; {} had stale HTML.",
                        result.scanned, result.rewritten
                    );
                    Ok(())
                })
            })
            .await
        }
        Some(other) => bail!(
            "unknown subcommand {other:?}; expected `seed`, `export [dir]`, `backup [dir]`, \
             `rerender`, `hash-password`, `version`, or no argument to serve"
        ),
    }
}

/// Opens the database, runs `body`, and closes the pools whether or not it
/// succeeded — SQLite checkpoints the WAL on close, and a subcommand that left a
/// hot journal behind would hand the next `serve` a recovery on startup.
async fn with_db<F>(cfg: config::Config, body: F) -> Result<()>
where
    F: for<'a> FnOnce(&'a db::Db) -> BoxFuture<'a, Result<()>>,
{
    let db = db::Db::connect(&cfg).await?;
    let result = body(&db).await;
    db.close().await;
    result
}

type BoxFuture<'a, T> = std::pin::Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Prints an argon2id PHC string for `YOUWIN_PASSWORD_HASH`.
///
/// Never reads argv — an argument would put the plaintext in `ps` output and
/// shell history. On a terminal it prompts without echoing; when stdin is a pipe
/// it reads two lines instead, so provisioning can be scripted.
fn hash_password() -> Result<()> {
    let (password, confirm) = if std::io::stdin().is_terminal() {
        (
            rpassword::prompt_password("New password: ")?,
            rpassword::prompt_password("Confirm: ")?,
        )
    } else {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        piped_credentials(&input)
    };

    if password != confirm {
        bail!("passwords do not match");
    }
    if password.chars().count() < 12 {
        bail!("use at least 12 characters — this is the only credential on the box");
    }

    // stdout, so `youwin-server hash-password > /etc/youwin/secrets.env` works;
    // the prompts above go to the tty.
    println!("YOUWIN_PASSWORD_HASH={}", password::hash(&password)?);
    Ok(())
}

/// Splits piped stdin into (password, confirmation).
///
/// Strips a leading byte-order mark. PowerShell prepends one when piping to a
/// native command, and hashing `U+FEFF` + the password produces a credential
/// that cannot be typed — the resulting login failure gives no clue why, because
/// the offending character is invisible in every tool you would reach for. A BOM
/// at the start of a password is never intentional.
///
/// A single-line pipe confirms itself: there is no second chance to mistype it.
fn piped_credentials(input: &str) -> (String, String) {
    let cleaned = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut lines = cleaned.lines();

    let password = lines.next().unwrap_or_default().to_owned();
    let confirm = lines
        .next()
        .filter(|line| !line.is_empty())
        .map_or_else(|| password.clone(), str::to_owned);

    (password, confirm)
}

#[cfg(test)]
mod tests {
    use super::piped_credentials;

    #[test]
    fn a_leading_bom_is_stripped() {
        let (password, confirm) = piped_credentials("\u{feff}hunter2hunter2\r\n");
        assert_eq!(password, "hunter2hunter2");
        assert_eq!(confirm, "hunter2hunter2");
    }

    #[test]
    fn one_line_confirms_itself_and_two_lines_are_compared() {
        assert_eq!(piped_credentials("secret\n"), ("secret".into(), "secret".into()));
        assert_eq!(piped_credentials("secret"), ("secret".into(), "secret".into()));
        // A trailing blank line is an artifact, not a failed confirmation.
        assert_eq!(piped_credentials("secret\n\n"), ("secret".into(), "secret".into()));

        let (password, confirm) = piped_credentials("one\ntwo\n");
        assert_ne!(password, confirm, "a genuine mismatch must still be caught");
    }

    #[test]
    fn crlf_line_endings_do_not_become_part_of_the_password() {
        let (password, confirm) = piped_credentials("secret\r\nsecret\r\n");
        assert_eq!(password, "secret");
        assert_eq!(confirm, "secret");
    }
}

async fn serve(cfg: config::Config) -> Result<()> {
    // Auth exists as of M2, so this is now a hard requirement rather than an
    // optional value. Failing here means a misconfigured deploy never starts;
    // the alternative is a running site whose login can never succeed.
    let Some(password_hash) = cfg.password_hash.clone() else {
        bail!(
            "YOUWIN_PASSWORD_HASH is not set. Generate one with `youwin-server hash-password` \
             and put it in the unit's EnvironmentFile."
        );
    };

    if !password_hash.starts_with("$argon2id$") {
        bail!("YOUWIN_PASSWORD_HASH is not an argon2id PHC string; regenerate it");
    }

    let db = db::Db::connect(&cfg).await?;

    // Expired rows are already unusable, but nothing else ever removes them.
    let purged = youwin_server::db::sessions::purge_expired(&db.write, now_millis()).await?;
    if purged > 0 {
        tracing::info!(purged, "removed expired sessions");
    }

    // Read once at startup. A missing manifest is fatal on purpose: the public
    // site cannot render without its stylesheet, and failing here says so
    // clearly rather than serving unstyled HTML that looks like a CSS bug.
    let assets = public::assets::Assets::load(&cfg.public_dist)?;

    let public_listener = TcpListener::bind(cfg.public_bind)
        .await
        .with_context(|| format!("binding public listener on {}", cfg.public_bind))?;
    let write_listener = TcpListener::bind(cfg.write_bind)
        .await
        .with_context(|| format!("binding authoring listener on {}", cfg.write_bind))?;

    tracing::info!(
        build = BUILD,
        public = %cfg.public_bind,
        authoring = %cfg.write_bind,
        database = %cfg.database_path.display(),
        stylesheet = %assets.css,
        "listening",
    );

    // Two listeners rather than one router branching on the Host header. The
    // public router has no authoring routes compiled into it and no handle to
    // the write pool, so the boundary is enforced at the socket — a routing or
    // middleware bug cannot expose the composer. See DESIGN.md "Shape".
    let public_router = public::router(
        db.read.clone(),
        assets.clone(),
        cfg.public_origin.clone(),
        &cfg.public_dist,
    );
    let public = axum::serve(public_listener, public_router).with_graceful_shutdown(shutdown_signal());
    let purger = youwin_server::cache::Purger::new(
        cfg.cf_zone_id.as_deref(),
        cfg.cf_purge_token.as_deref(),
        &cfg.cf_api_base,
    );
    tracing::info!(
        cache_purging = if purger.is_enabled() { "on" } else { "off (TTL only)" },
        "edge cache",
    );

    let authoring_router = write::router(
        db.clone(),
        write::AuthConfig {
            password_hash,
            cookie_secure: cfg.cookie_secure,
            origin: cfg.write_origin.clone(),
        },
        assets.clone(),
        cfg.public_origin.clone(),
        purger,
    );
    let authoring =
        axum::serve(write_listener, authoring_router).with_graceful_shutdown(shutdown_signal());

    // Each server gets its own shutdown future. tokio supports multiple
    // concurrent listeners on the same signal, so there is no fan-out channel to
    // plumb and get wrong.
    tokio::try_join!(public.into_future(), authoring.into_future())?;

    db.close().await;
    tracing::info!("shutdown complete");
    Ok(())
}

/// Resolves on Ctrl+C or SIGTERM. systemd sends SIGTERM on restart; without
/// handling it the process is killed mid-commit and leaves a hot WAL behind.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    // Windows is the dev platform only; there is no SIGTERM to wait for, so this
    // arm simply never resolves.
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
