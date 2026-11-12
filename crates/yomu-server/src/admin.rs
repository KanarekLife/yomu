//! Server-rendered admin panel under `/admin`. Plain HTML forms, no JS.
//! CSRF protection relies on the `SameSite=Strict` session cookie. Panel
//! admins (OIDC or the env break-glass login) are not app users.

// Handlers return `Result<Response, Response>`; boxing the error buys nothing.
#![allow(clippy::result_large_err)]

use axum::{
    extract::{FromRequestParts, Path, Query, State},
    http::{header, request::Parts, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use serde::Deserialize;

use std::time::Duration;

use crate::{
    auth,
    oidc::{self, AuthRequest, OidcError},
    AppState,
};

const COOKIE: &str = "yomu_admin";
/// Holds state/nonce/PKCE verifier between redirect and callback.
const OIDC_COOKIE: &str = "yomu_oidc";
const SESSION_DAYS: i64 = 7;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(dashboard))
        .route("/login", get(login_page).post(login_submit))
        .route("/logout", get(logout).post(logout))
        .route("/oidc/start", get(oidc_start))
        .route("/oidc/callback", get(oidc_callback))
        .route("/users", post(create_user))
        .route("/users/{id}", get(user_page))
        .route("/users/{id}/password", post(reset_password))
        .route("/users/{id}/toggle-disabled", post(toggle_disabled))
        .route("/users/{id}/delete", post(delete_user))
        .route("/users/{id}/revoke", post(revoke_token))
}

// ---------- helpers ----------

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn human_bytes(n: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// Minimal percent-encoding for flash messages in redirect URLs.
fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn back(path: &str, msg: Result<&str, &str>) -> Response {
    let q = match msg {
        Ok(m) => format!("msg={}", urlencode(m)),
        Err(e) => format!("err={}", urlencode(e)),
    };
    let sep = if path.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{path}{sep}{q}")).into_response()
}

fn page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title} · Yomu admin</title>
<style>
:root{{color-scheme:light dark;--b:#8884}}
body{{font-family:system-ui,sans-serif;max-width:60rem;margin:0 auto;padding:1rem;line-height:1.4}}
header{{display:flex;justify-content:space-between;align-items:center;gap:1rem;flex-wrap:wrap}}
table{{border-collapse:collapse;width:100%}}
.scroll{{overflow-x:auto}}
th,td{{text-align:left;padding:.4rem .5rem;border-bottom:1px solid var(--b);white-space:nowrap}}
form.inline{{display:inline}}
fieldset{{border:1px solid var(--b);border-radius:.5rem;margin:1rem 0}}
input[type=text],input[type=password]{{padding:.4rem;max-width:100%}}
button{{padding:.35rem .8rem;cursor:pointer}}
.msg{{padding:.5rem 1rem;border-radius:.4rem;background:#2a72;}}
.err{{padding:.5rem 1rem;border-radius:.4rem;background:#c332;}}
.danger{{color:#c33}}
</style></head><body>{body}</body></html>"#,
        title = escape(title),
    ))
}

#[derive(Deserialize, Default)]
struct Flash {
    msg: Option<String>,
    err: Option<String>,
}

impl Flash {
    fn html(&self) -> String {
        let mut s = String::new();
        if let Some(m) = &self.msg {
            s += &format!(r#"<p class="msg">{}</p>"#, escape(m));
        }
        if let Some(e) = &self.err {
            s += &format!(r#"<p class="err">{}</p>"#, escape(e));
        }
        s
    }
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

fn internal(e: impl std::fmt::Debug) -> Response {
    tracing::error!(error = ?e, "admin panel error");
    (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
}

type PageResult = Result<Response, Response>;

// ---------- session extractor ----------

/// A signed-in panel admin. Admins are not app users: the session names an
/// identity such as `oidc:<sub> (<name>)` or `local:<name>`.
pub struct Admin {
    pub identity: String,
}

/// Whether a stored session identity is still allowed by the current config
/// (disabling a method, or renaming the local admin, ends those sessions).
fn identity_allowed(state: &AppState, identity: &str) -> bool {
    if identity.starts_with("oidc:") {
        state.oidc.is_some()
    } else if let Some(name) = identity.strip_prefix("local:") {
        state
            .config
            .local_admin
            .as_ref()
            .is_some_and(|l| l.username == name)
    } else {
        false
    }
}

impl FromRequestParts<AppState> for Admin {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        let to_login = || Redirect::to("/admin/login").into_response();
        let sid = cookie_value(&parts.headers, COOKIE).ok_or_else(to_login)?;
        let client = state.pool.get().await.map_err(internal)?;
        let identity: String = client
            .query_opt(
                "SELECT admin FROM admin_sessions WHERE id_hash = $1 AND expires_at > now()",
                &[&auth::hash_secret(&sid)],
            )
            .await
            .map_err(internal)?
            .ok_or_else(to_login)?
            .get(0);
        if !identity_allowed(state, &identity) {
            return Err(to_login());
        }
        Ok(Admin { identity })
    }
}

/// Stores a new session for `identity` and redirects to the dashboard.
async fn start_session(state: &AppState, identity: &str) -> PageResult {
    let secret = auth::generate_secret();
    let client = state.pool.get().await.map_err(internal)?;
    client
        .execute("DELETE FROM admin_sessions WHERE expires_at < now()", &[])
        .await
        .map_err(internal)?;
    client
        .execute(
            "INSERT INTO admin_sessions (id_hash, admin, expires_at)
             VALUES ($1, $2, now() + make_interval(days => $3::int))",
            &[&secret.hash, &identity, &(SESSION_DAYS as i32)],
        )
        .await
        .map_err(internal)?;
    tracing::info!(admin = identity, "admin signed in");
    let cookie = format!(
        "{COOKIE}={}; HttpOnly; Secure; SameSite=Strict; Path=/admin; Max-Age={}",
        secret.token,
        SESSION_DAYS * 86400
    );
    let mut resp = Redirect::to("/admin").into_response();
    resp.headers_mut()
        .append(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    Ok(resp)
}

// ---------- login / logout ----------

async fn login_page(State(state): State<AppState>, Query(flash): Query<Flash>) -> Html<String> {
    let mut body = format!("<h1>Yomu admin</h1>{}", flash.html());
    if state.oidc.is_some() {
        body += r#"<p><a href="/admin/oidc/start"><button type="button">Sign in with SSO</button></a></p>"#;
    }
    if state.config.local_admin.is_some() {
        if state.oidc.is_some() {
            body += "<p><small>or use the local admin account:</small></p>";
        }
        body += r#"<form method="post" action="/admin/login">
<p><label>Username<br><input type="text" name="username" autocomplete="username" required></label></p>
<p><label>Password<br><input type="password" name="password" autocomplete="current-password" required></label></p>
<p><button type="submit">Log in</button></p>
</form>"#;
    }
    if state.oidc.is_none() && state.config.local_admin.is_none() {
        body += r#"<p class="err">The admin panel is disabled: no sign-in method is configured.
Set <code>YOMU_OIDC_ISSUER</code>/<code>YOMU_OIDC_CLIENT_ID</code>/<code>YOMU_OIDC_CLIENT_SECRET</code>
or <code>YOMU_ADMIN_USER</code>/<code>YOMU_ADMIN_PASSWORD</code>.</p>"#;
    }
    body += r#"<p><small>App accounts cannot sign in here.</small></p><p><a href="/">&larr; back</a></p>"#;
    page("Log in", &body)
}

#[derive(Deserialize)]
struct LoginForm {
    username: String,
    password: String,
}

async fn login_submit(State(state): State<AppState>, Form(f): Form<LoginForm>) -> PageResult {
    let Some(local) = &state.config.local_admin else {
        return Ok(back("/admin/login", Err("Local admin login is disabled.")));
    };
    let name = auth::normalize_username(&f.username);
    // Non-short-circuiting `&`: both comparisons always run.
    let ok =
        auth::secret_eq(&name, &local.username) & auth::secret_eq(&f.password, &local.password);
    if !ok {
        tracing::warn!("failed local admin login");
        tokio::time::sleep(Duration::from_secs(1)).await;
        return Ok(back("/admin/login", Err("Invalid credentials.")));
    }
    start_session(&state, &format!("local:{}", local.username)).await
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> PageResult {
    if let Some(sid) = cookie_value(&headers, COOKIE) {
        let client = state.pool.get().await.map_err(internal)?;
        client
            .execute(
                "DELETE FROM admin_sessions WHERE id_hash = $1",
                &[&auth::hash_secret(&sid)],
            )
            .await
            .map_err(internal)?;
    }
    let mut resp = back("/admin/login", Ok("Logged out."));
    resp.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "yomu_admin=; HttpOnly; Secure; SameSite=Strict; Path=/admin; Max-Age=0",
        ),
    );
    Ok(resp)
}

// ---------- OIDC ----------

fn clear_oidc_cookie(resp: &mut Response) {
    resp.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "yomu_oidc=; HttpOnly; Secure; SameSite=Lax; Path=/admin/oidc; Max-Age=0",
        ),
    );
}

fn oidc_error_page(status: StatusCode, msg: &str) -> Response {
    let body = format!(
        r#"<h1>Sign-in failed</h1><p class="err">{}</p><p><a href="/admin/login">Back to login</a></p>"#,
        escape(msg)
    );
    let mut resp = (status, page("Sign-in failed", &body)).into_response();
    clear_oidc_cookie(&mut resp);
    resp
}

async fn oidc_start(State(state): State<AppState>) -> Response {
    let Some(oidc) = &state.oidc else {
        return oidc_error_page(StatusCode::NOT_FOUND, "SSO is not configured.");
    };
    // Fail closed: no discovery, no sign-in.
    let disc = match oidc.discovery().await {
        Ok(d) => d,
        Err(e) => {
            tracing::error!(error = %e, "OIDC discovery failed");
            return oidc_error_page(
                StatusCode::BAD_GATEWAY,
                &format!("The identity provider is unavailable: {e}"),
            );
        }
    };
    let req = AuthRequest::generate();
    let location = oidc::authorization_url(&disc, &oidc.config, &req);
    let cookie = format!(
        "{OIDC_COOKIE}={}; HttpOnly; Secure; SameSite=Lax; Path=/admin/oidc; Max-Age=600",
        req.to_cookie()
    );
    let mut resp = Redirect::to(&location).into_response();
    resp.headers_mut()
        .append(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    resp
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

async fn oidc_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> Response {
    let Some(oidc) = &state.oidc else {
        return oidc_error_page(StatusCode::NOT_FOUND, "SSO is not configured.");
    };
    if let Some(err) = q.error {
        let desc = q.error_description.unwrap_or_default();
        return oidc_error_page(
            StatusCode::BAD_REQUEST,
            &format!("The identity provider refused the sign-in: {err} {desc}"),
        );
    }
    let Some(req) = cookie_value(&headers, OIDC_COOKIE).and_then(|c| AuthRequest::from_cookie(&c))
    else {
        return oidc_error_page(
            StatusCode::BAD_REQUEST,
            "Sign-in session expired or missing; please start again.",
        );
    };
    let (Some(code), Some(returned_state)) = (q.code, q.state) else {
        return oidc_error_page(StatusCode::BAD_REQUEST, "Missing code or state.");
    };
    match oidc.complete(&req, &code, &returned_state).await {
        Ok(identity) => {
            let mut resp = match start_session(&state, &identity).await {
                Ok(r) | Err(r) => r,
            };
            clear_oidc_cookie(&mut resp);
            resp
        }
        Err(OidcError::BadRequest(m)) => oidc_error_page(StatusCode::BAD_REQUEST, &m),
        Err(OidcError::Denied(m)) => {
            tracing::warn!(reason = %m, "OIDC sign-in denied");
            oidc_error_page(StatusCode::FORBIDDEN, &format!("Access denied: {m}"))
        }
        Err(OidcError::Provider(m)) => {
            tracing::error!(error = %m, "OIDC sign-in failed");
            oidc_error_page(StatusCode::BAD_GATEWAY, &m)
        }
    }
}

// ---------- dashboard ----------

async fn dashboard(
    State(state): State<AppState>,
    admin: Admin,
    Query(flash): Query<Flash>,
) -> PageResult {
    let client = state.pool.get().await.map_err(internal)?;
    let rows = client
        .query(
            "SELECT u.id, u.username, u.disabled,
                    (SELECT count(*) FROM books b WHERE b.user_id = u.id AND NOT b.deleted),
                    (SELECT COALESCE(sum(size), 0)::int8 FROM files f WHERE f.user_id = u.id)
                  + (SELECT COALESCE(sum(octet_length(t.text) + octet_length(t.key) + octet_length(t.lang) + 48), 0)::int8
                       FROM translations t WHERE t.user_id = u.id),
                    (SELECT count(*) FROM tokens k WHERE k.user_id = u.id),
                    to_char(u.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD')
             FROM users u ORDER BY u.username",
            &[],
        )
        .await
        .map_err(internal)?;
    let mut table = String::new();
    for r in &rows {
        let id: i64 = r.get(0);
        let name: String = r.get(1);
        table += &format!(
            r#"<tr><td><a href="/admin/users/{id}">{}</a></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
            escape(&name),
            if r.get(2) { "yes" } else { "" },
            r.get::<_, i64>(3),
            human_bytes(r.get(4)),
            r.get::<_, i64>(5),
            escape(&r.get::<_, String>(6)),
        );
    }
    if rows.is_empty() {
        table = r#"<tr><td colspan="6">No users yet.</td></tr>"#.into();
    }
    let body = format!(
        r#"<header><h1>Yomu admin</h1><span>{me} · <a href="/admin/logout">log out</a></span></header>
{flash}
<h2>App users</h2>
<div class="scroll"><table>
<tr><th>Username</th><th>Disabled</th><th>Books</th><th>Storage</th><th>Devices</th><th>Created</th></tr>
{table}</table></div>
<fieldset><legend>Create user</legend>
<form method="post" action="/admin/users">
<p><label>Username<br><input type="text" name="username" required autocomplete="off"></label></p>
<p><label>Password<br><input type="password" name="password" required minlength="8" autocomplete="new-password"></label></p>
<p><button type="submit">Create</button></p>
</form></fieldset>
<p><small>Admins are separate from app users; to use the app yourself, create a user for it. Yomu sync server {ver}</small></p>"#,
        me = escape(&admin.identity),
        flash = flash.html(),
        ver = env!("CARGO_PKG_VERSION"),
    );
    Ok(page("Users", &body).into_response())
}

#[derive(Deserialize)]
struct CreateUserForm {
    username: String,
    password: String,
}

async fn create_user(
    State(state): State<AppState>,
    admin: Admin,
    Form(f): Form<CreateUserForm>,
) -> PageResult {
    let username = auth::normalize_username(&f.username);
    if let Err(e) = auth::validate_username(&username).and(auth::validate_password(&f.password)) {
        return Ok(back("/admin", Err(e)));
    }
    let hash = auth::hash_password(f.password).await;
    let client = state.pool.get().await.map_err(internal)?;
    let n = client
        .execute(
            "INSERT INTO users (username, password_hash) VALUES ($1, $2)
             ON CONFLICT (username) DO NOTHING",
            &[&username, &hash],
        )
        .await
        .map_err(internal)?;
    if n == 0 {
        return Ok(back("/admin", Err("That username is taken.")));
    }
    tracing::info!(admin = admin.identity, username, "user created");
    Ok(back("/admin", Ok(&format!("Created user {username}."))))
}

// ---------- per-user page ----------

struct UserRow {
    username: String,
    disabled: bool,
}

async fn load_user(client: &tokio_postgres::Client, id: i64) -> Result<Option<UserRow>, Response> {
    Ok(client
        .query_opt("SELECT username, disabled FROM users WHERE id = $1", &[&id])
        .await
        .map_err(internal)?
        .map(|r| UserRow {
            username: r.get(0),
            disabled: r.get(1),
        }))
}

async fn user_page(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<i64>,
    Query(flash): Query<Flash>,
) -> PageResult {
    let client = state.pool.get().await.map_err(internal)?;
    let Some(u) = load_user(&client, id).await? else {
        return Ok(back("/admin", Err("No such user.")));
    };
    let tokens = client
        .query(
            "SELECT token_hash, device_name,
                    to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI'),
                    to_char(last_used_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI')
             FROM tokens WHERE user_id = $1 ORDER BY last_used_at DESC",
            &[&id],
        )
        .await
        .map_err(internal)?;
    let mut devices = String::new();
    for t in &tokens {
        let h: Vec<u8> = t.get(0);
        devices += &format!(
            r#"<tr><td>{}</td><td>{}</td><td>{}</td><td><form class="inline" method="post" action="/admin/users/{id}/revoke"><input type="hidden" name="token" value="{}"><button type="submit">Revoke</button></form></td></tr>"#,
            escape(&t.get::<_, String>(1)),
            escape(&t.get::<_, String>(2)),
            escape(&t.get::<_, String>(3)),
            hex::encode(h),
        );
    }
    if tokens.is_empty() {
        devices = r#"<tr><td colspan="4">No devices signed in.</td></tr>"#.into();
    }
    let name = escape(&u.username);
    let body = format!(
        r#"<header><h1>{name}</h1><span><a href="/admin">&larr; all users</a> · <a href="/admin/logout">log out</a></span></header>
{flash}
<p>Disabled: <b>{dis}</b></p>
<h2>Devices</h2>
<div class="scroll"><table><tr><th>Device</th><th>Signed in (UTC)</th><th>Last used (UTC)</th><th></th></tr>{devices}</table></div>
<fieldset><legend>Reset password</legend>
<form method="post" action="/admin/users/{id}/password">
<p><input type="password" name="password" required minlength="8" autocomplete="new-password" placeholder="new password"></p>
<p><label><input type="checkbox" name="revoke" value="on" checked> Sign out all devices</label></p>
<p><button type="submit">Set password</button></p></form></fieldset>
<fieldset><legend>Status</legend>
<form class="inline" method="post" action="/admin/users/{id}/toggle-disabled"><button type="submit">{dis_btn}</button></form>
</fieldset>
<fieldset><legend class="danger">Delete user</legend>
<form method="post" action="/admin/users/{id}/delete">
<p>Deletes the account with all books, settings, translations and files. Type the username to confirm.</p>
<p><input type="text" name="confirm" required autocomplete="off" placeholder="{name}"></p>
<p><button type="submit" class="danger">Delete permanently</button></p></form></fieldset>"#,
        flash = flash.html(),
        dis = if u.disabled { "yes" } else { "no" },
        dis_btn = if u.disabled {
            "Enable account"
        } else {
            "Disable account"
        },
    );
    Ok(page(&u.username, &body).into_response())
}

#[derive(Deserialize)]
struct PasswordForm {
    password: String,
    revoke: Option<String>,
}

async fn reset_password(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<i64>,
    Form(f): Form<PasswordForm>,
) -> PageResult {
    let path = format!("/admin/users/{id}");
    if let Err(e) = auth::validate_password(&f.password) {
        return Ok(back(&path, Err(e)));
    }
    let hash = auth::hash_password(f.password).await;
    let client = state.pool.get().await.map_err(internal)?;
    let n = client
        .execute(
            "UPDATE users SET password_hash = $2 WHERE id = $1",
            &[&id, &hash],
        )
        .await
        .map_err(internal)?;
    if n == 0 {
        return Ok(back("/admin", Err("No such user.")));
    }
    if f.revoke.is_some() {
        client
            .execute("DELETE FROM tokens WHERE user_id = $1", &[&id])
            .await
            .map_err(internal)?;
    }
    Ok(back(&path, Ok("Password updated.")))
}

async fn toggle_disabled(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<i64>,
) -> PageResult {
    let client = state.pool.get().await.map_err(internal)?;
    let n = client
        .execute(
            "UPDATE users SET disabled = NOT disabled WHERE id = $1",
            &[&id],
        )
        .await
        .map_err(internal)?;
    if n == 0 {
        return Ok(back("/admin", Err("No such user.")));
    }
    Ok(back(
        &format!("/admin/users/{id}"),
        Ok("Account status changed."),
    ))
}

#[derive(Deserialize)]
struct DeleteForm {
    confirm: String,
}

async fn delete_user(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<i64>,
    Form(f): Form<DeleteForm>,
) -> PageResult {
    let path = format!("/admin/users/{id}");
    let client = state.pool.get().await.map_err(internal)?;
    let Some(u) = load_user(&client, id).await? else {
        return Ok(back("/admin", Err("No such user.")));
    };
    if u.username != auth::normalize_username(&f.confirm) {
        return Ok(back(&path, Err("Confirmation did not match the username.")));
    }
    client
        .execute("DELETE FROM users WHERE id = $1", &[&id])
        .await
        .map_err(internal)?;
    tracing::info!(
        admin = admin.identity,
        username = u.username,
        "user deleted"
    );
    Ok(back("/admin", Ok("User deleted.")))
}

#[derive(Deserialize)]
struct RevokeForm {
    token: String,
}

async fn revoke_token(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<i64>,
    Form(f): Form<RevokeForm>,
) -> PageResult {
    let path = format!("/admin/users/{id}");
    let Ok(hash) = hex::decode(f.token.trim()) else {
        return Ok(back(&path, Err("Bad token id.")));
    };
    let client = state.pool.get().await.map_err(internal)?;
    let n = client
        .execute(
            "DELETE FROM tokens WHERE user_id = $1 AND token_hash = $2",
            &[&id, &hash],
        )
        .await
        .map_err(internal)?;
    Ok(back(
        &path,
        if n > 0 {
            Ok("Device signed out.")
        } else {
            Err("Device not found.")
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html() {
        assert_eq!(
            escape(r#"<a href="x">'&'</a>"#),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
    }

    #[test]
    fn formats_bytes() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(200 * 1024 * 1024), "200.0 MiB");
    }

    #[test]
    fn parses_cookies() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, "a=1; yomu_admin=abc; b=2".parse().unwrap());
        assert_eq!(cookie_value(&h, COOKIE).as_deref(), Some("abc"));
        assert_eq!(cookie_value(&h, "c"), None);
    }
}
