use std::time::Duration;

use tracing_subscriber::EnvFilter;
use yomu_server::{app, db, AppState, Config};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tokio_postgres=warn")),
        )
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .init();
    if let Err(e) = run().await {
        tracing::error!("fatal: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let config = Config::from_env()?;
    let pool = db::make_pool(&config)?;
    // Startup may wait minutes for a cold database; stay responsive to SIGTERM.
    let startup = async {
        db::wait_for_db(&pool, Duration::from_secs(300)).await?;
        db::migrate(&pool).await
    };
    tokio::select! {
        r = startup => r?,
        _ = shutdown() => return Ok(()),
    }

    let listen = config.listen;
    match (&config.oidc, &config.local_admin) {
        (None, None) => tracing::warn!(
            "admin panel disabled: configure YOMU_OIDC_* or YOMU_ADMIN_USER/YOMU_ADMIN_PASSWORD"
        ),
        (oidc, local) => tracing::info!(
            oidc = oidc.as_ref().map(|o| o.issuer.as_str()),
            local_admin = local.is_some(),
            "admin panel sign-in"
        ),
    }
    let state = AppState::new(pool, config);
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|e| format!("cannot listen on {listen}: {e}"))?;
    tracing::info!(%listen, version = env!("CARGO_PKG_VERSION"), "yomu-server listening");
    axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(|e| e.to_string())
}

async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
    tracing::info!("shutting down");
}
