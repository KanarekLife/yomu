//! Yomu sync server: accounts, last-writer-wins sync of library/settings/
//! translations, EPUB blob storage, and a small server-rendered admin panel.

pub mod admin;
pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod sync;

use std::sync::Arc;

use axum::{response::Html, routing::get, Router};
use tower_http::trace::TraceLayer;

pub use config::Config;

#[derive(Clone)]
pub struct AppState {
    pub pool: deadpool_postgres::Pool,
    pub config: Arc<Config>,
}

impl AppState {
    /// A pooled connection, mapped to a 503/500 API error on failure.
    pub async fn db(&self) -> error::ApiResult<deadpool_postgres::Object> {
        Ok(self.pool.get().await?)
    }
}

/// The complete HTTP application.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(landing))
        .route("/healthz", get(api::healthz))
        .nest("/api/v1", api::router(&state.config))
        .nest("/admin", admin::router())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn landing() -> Html<&'static str> {
    Html(concat!(
        "<!doctype html><html><head><meta charset=\"utf-8\">",
        "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">",
        "<title>Yomu sync server</title>",
        "<style>body{font-family:system-ui,sans-serif;max-width:40rem;margin:3rem auto;padding:0 1rem;line-height:1.5}</style>",
        "</head><body><h1>Yomu sync server</h1>",
        "<p>This server keeps your Yomu library, reading positions, settings and translations in sync across devices.</p>",
        "<p><a href=\"/admin\">Admin panel</a> &middot; <a href=\"https://github.com/KanarekLife/yomu\">Yomu on GitHub</a></p>",
        "<p><small>version ",
        env!("CARGO_PKG_VERSION"),
        "</small></p></body></html>"
    ))
}
