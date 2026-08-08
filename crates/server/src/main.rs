use std::future::IntoFuture;

use anyhow::{Context as _, Result, bail};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;
use youwin_server::{config, db, public, seed, write};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "youwin_server=info,tower_http=warn".into()),
        )
        .init();

    let cfg = config::Config::from_env()?;

    // Hand-rolled rather than a CLI crate: there is one subcommand, and adding
    // an argument parser for it would be more code than the feature.
    match std::env::args().nth(1).as_deref() {
        None => serve(cfg).await,
        Some("seed") => {
            let db = db::Db::connect(&cfg).await?;
            let result = seed::run(&db).await;
            db.close().await;
            result
        }
        Some(other) => bail!("unknown subcommand {other:?}; expected `seed`, or no argument to serve"),
    }
}

async fn serve(cfg: config::Config) -> Result<()> {
    let db = db::Db::connect(&cfg).await?;

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
        assets,
        cfg.public_origin.clone(),
        &cfg.public_dist,
    );
    let public = axum::serve(public_listener, public_router).with_graceful_shutdown(shutdown_signal());
    let authoring = axum::serve(write_listener, write::router(db.clone()))
        .with_graceful_shutdown(shutdown_signal());

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
