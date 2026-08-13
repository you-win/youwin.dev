//! `youwin-offsite` — receives the nightly backup on a box that is not the one
//! being backed up. See `lib.rs` for why it exists at all, and
//! `deploy/offsite/README.md` for how it is provisioned.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;
use youwin_offsite::{config::Config, http, name::Artifact};

/// Identifies the build. This one is installed by hand rather than by CI — see
/// the deploy README — which makes "what is actually on the box?" a question
/// somebody will genuinely have to ask.
const BUILD: &str = match option_env!("YOUWIN_BUILD") {
    Some(build) => build,
    None => "dev",
};

#[tokio::main]
async fn main() -> Result<()> {
    // Answered before anything else is touched — no logging, no environment, no
    // filesystem. After a manual `scp` this is how you confirm what landed, so
    // it has to work on a box where nothing is configured yet.
    if matches!(
        std::env::args().nth(1).as_deref(),
        Some("version" | "--version" | "-V")
    ) {
        println!("youwin-offsite {} ({BUILD})", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "youwin_offsite=info".into()),
        )
        .init();

    let cfg = Config::from_env()?;

    // Proven writable now rather than at 3am. `create_dir_all` succeeding says
    // nothing about whether this user may write *into* the directory — an
    // existing root-owned one passes it and then fails on the first upload, at
    // the hour and in the place where the failure is least welcome.
    let probe = cfg.dir.join(".youwin-offsite-writable");
    std::fs::create_dir_all(&cfg.dir)
        .with_context(|| format!("creating {}", cfg.dir.display()))?;
    std::fs::write(&probe, b"")
        .and_then(|()| std::fs::remove_file(&probe))
        .with_context(|| format!("{} is not writable by this user", cfg.dir.display()))?;

    let listener = TcpListener::bind(cfg.bind)
        .await
        .with_context(|| format!("binding on {}", cfg.bind))?;

    let (stored, newest) = survey(&cfg.dir);
    tracing::info!(
        build = BUILD,
        bind = %cfg.bind,
        dir = %cfg.dir.display(),
        keep = cfg.keep,
        max_bytes = cfg.max_bytes,
        stored,
        newest = newest.as_deref().unwrap_or("none"),
        "listening",
    );

    axum::serve(listener, http::router(Arc::new(http::Receiver::new(&cfg))))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving")?;

    tracing::info!("shutdown complete");
    Ok(())
}

/// How many backups are already here, and the newest date among them.
///
/// Logged at startup because there is no health endpoint to ask — the Caddy
/// block aborts everything but `PUT`. A restart therefore prints the one line
/// that would otherwise need an `ls`, and a `newest` that is not yesterday is
/// visible in `systemctl status` without anything having to raise an alarm
/// about it.
fn survey(dir: &std::path::Path) -> (usize, Option<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, None);
    };

    let dates: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            Artifact::parse(&entry.file_name().to_string_lossy())
                .map(|artifact| artifact.date().to_owned())
        })
        .collect();

    // Zero-padded and big-endian, so lexical order is chronological.
    let newest = dates.iter().max().cloned();
    (dates.len(), newest)
}

/// Resolves on Ctrl+C or SIGTERM, so systemd's restart does not sever an upload
/// mid-stream and leave a `.part` behind on every deploy of the *other* box.
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

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
