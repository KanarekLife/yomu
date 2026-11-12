//! JSON API under `/api/v1`.

use axum::{
    body::Bytes,
    extract::{rejection::JsonRejection, DefaultBodyLimit, Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use tower_http::{cors::CorsLayer, limit::RequestBodyLimitLayer};

use crate::{
    auth::{self, AuthUser},
    error::{ApiError, ApiResult},
    sync, AppState, Config,
};

/// Sync payloads can carry thousands of translations.
const SYNC_BODY_LIMIT: usize = 64 * 1024 * 1024;

pub fn router(config: &Config) -> Router<AppState> {
    let files = Router::new()
        .route("/files/{key}", put(put_file).get(get_file))
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(config.max_file_bytes));
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
        .route("/version", get(version))
        .route(
            "/sync",
            post(sync_handler).layer(DefaultBodyLimit::max(SYNC_BODY_LIMIT)),
        )
        .merge(files)
        .fallback(|| async { ApiError::not_found("no such endpoint") })
        .layer(axum::middleware::map_response(json_payload_too_large))
        .layer(CorsLayer::permissive())
}

/// The body-limit layers answer 413 with plain text; keep the API all-JSON.
async fn json_payload_too_large(resp: axum::response::Response) -> axum::response::Response {
    let is_json = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|v| v.as_bytes().starts_with(b"application/json"));
    if resp.status() == StatusCode::PAYLOAD_TOO_LARGE && !is_json {
        return ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "request body too large")
            .into_response();
    }
    resp
}

fn json_body<T>(body: Result<Json<T>, JsonRejection>) -> ApiResult<T> {
    body.map(|Json(v)| v)
        .map_err(|e| ApiError::new(e.status(), e.body_text()))
}

pub async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    let ok = match state.pool.get().await {
        Ok(c) => c.simple_query("SELECT 1").await.is_ok(),
        Err(_) => false,
    };
    if ok {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "database unavailable")
    }
}

async fn version() -> Json<serde_json::Value> {
    Json(json!({ "version": env!("CARGO_PKG_VERSION") }))
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
    #[serde(default)]
    device_name: String,
}

async fn login(
    State(state): State<AppState>,
    body: Result<Json<LoginRequest>, JsonRejection>,
) -> ApiResult<Json<serde_json::Value>> {
    let req = json_body(body)?;
    let username = auth::normalize_username(&req.username);
    let client = state.db().await?;
    let row = client
        .query_opt(
            "SELECT id, password_hash, disabled FROM users WHERE username = $1",
            &[&username],
        )
        .await?;
    let hash = row.as_ref().map(|r| r.get::<_, String>(1));
    let password_ok = auth::verify_password(req.password, hash).await;
    let row = match row {
        Some(r) if password_ok && !r.get::<_, bool>(2) => r,
        _ => return Err(ApiError::unauthorized("invalid username or password")),
    };
    let user_id: i64 = row.get(0);
    let device: String = {
        let d = req.device_name.trim();
        let d = if d.is_empty() { "unknown device" } else { d };
        d.chars().take(128).collect()
    };
    let secret = auth::generate_secret();
    client
        .execute(
            "INSERT INTO tokens (token_hash, user_id, device_name) VALUES ($1, $2, $3)",
            &[&secret.hash, &user_id, &device],
        )
        .await?;
    tracing::info!(username, device, "login");
    Ok(Json(json!({
        "token": secret.token,
        "username": username,
    })))
}

async fn logout(State(state): State<AppState>, user: AuthUser) -> ApiResult<StatusCode> {
    state
        .db()
        .await?
        .execute(
            "DELETE FROM tokens WHERE token_hash = $1",
            &[&user.token_hash],
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn me(user: AuthUser) -> Json<serde_json::Value> {
    Json(json!({ "username": user.username }))
}

async fn sync_handler(
    State(state): State<AppState>,
    user: AuthUser,
    body: Result<Json<sync::SyncRequest>, JsonRejection>,
) -> ApiResult<Json<sync::SyncResponse>> {
    let req = json_body(body)?;
    let mut client = state.db().await?;
    Ok(Json(sync::run(&mut client, user.user_id, req).await?))
}

fn check_file_key(key: &str) -> ApiResult<()> {
    if key.is_empty() || key.len() > 512 {
        return Err(ApiError::bad_request("file key must be 1-512 bytes"));
    }
    Ok(())
}

async fn put_file(
    State(state): State<AppState>,
    user: AuthUser,
    Path(key): Path<String>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    check_file_key(&key)?;
    let size = body.len() as i64;
    state
        .db()
        .await?
        .execute(
            "INSERT INTO files (user_id, key, data, size, uploaded_at) VALUES ($1, $2, $3, $4, now())
             ON CONFLICT (user_id, key) DO UPDATE
               SET data = EXCLUDED.data, size = EXCLUDED.size, uploaded_at = now()",
            &[&user.user_id, &key, &body.as_ref(), &size],
        )
        .await?;
    tracing::info!(user = user.username, key, size, "file stored");
    Ok(StatusCode::NO_CONTENT)
}

async fn get_file(
    State(state): State<AppState>,
    user: AuthUser,
    Path(key): Path<String>,
) -> ApiResult<impl IntoResponse> {
    check_file_key(&key)?;
    let row = state
        .db()
        .await?
        .query_opt(
            "SELECT data FROM files WHERE user_id = $1 AND key = $2",
            &[&user.user_id, &key],
        )
        .await?
        .ok_or_else(|| ApiError::not_found("file not found"))?;
    let data: Vec<u8> = row.get(0);
    Ok(([(header::CONTENT_TYPE, "application/epub+zip")], data))
}
