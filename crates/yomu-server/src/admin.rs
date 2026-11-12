//! Server-rendered admin panel under `/admin`. Plain HTML forms, no JS.
//! CSRF protection relies on the `SameSite=Strict` session cookie.

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

use crate::{auth, AppState};

const COOKIE: &str = "yomu_admin";
const SESSION_DAYS: i64 = 7;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(dashboard))
        .route("/login", get(login_page).post(login_submit))
        .route("/logout", get(logout).post(logout))
        .route("/users", post(create_user))
        .route("/users/{id}", get(user_page))
        .route("/users/{id}/password", post(reset_password))
        .route("/users/{id}/toggle-disabled", post(toggle_disabled))
        .route("/users/{id}/toggle-admin", post(toggle_admin))
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

fn internal(e: impl std::fmt::Display) -> Response {
    tracing::error!(error = %e, "admin panel error");
    (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
}

type PageResult = Result<Response, Response>;

// ---------- session extractor ----------

/// A logged-in, enabled admin. Anything else is redirected to the login page.
pub struct Admin {
    pub user_id: i64,
    pub username: String,
}

impl FromRequestParts<AppState> for Admin {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        let to_login = || Redirect::to("/admin/login").into_response();
        let sid = cookie_value(&parts.headers, COOKIE).ok_or_else(to_login)?;
        let client = state.pool.get().await.map_err(internal)?;
        let row = client
            .query_opt(
                "SELECT u.id, u.username FROM admin_sessions s JOIN users u ON u.id = s.user_id
                 WHERE s.id_hash = $1 AND s.expires_at > now() AND u.is_admin AND NOT u.disabled",
                &[&auth::hash_secret(&sid)],
            )
            .await
            .map_err(internal)?
            .ok_or_else(to_login)?;
        Ok(Admin {
            user_id: row.get(0),
            username: row.get(1),
        })
    }
}

// ---------- login / logout ----------

async fn login_page(Query(flash): Query<Flash>) -> Html<String> {
    page(
        "Log in",
        &format!(
            r#"<h1>Yomu admin</h1>{}
<form method="post" action="/admin/login">
<p><label>Username<br><input type="text" name="username" autocomplete="username" required autofocus></label></p>
<p><label>Password<br><input type="password" name="password" autocomplete="current-password" required></label></p>
<p><button type="submit">Log in</button></p>
</form><p><a href="/">&larr; back</a></p>"#,
            flash.html()
        ),
    )
}

#[derive(Deserialize)]
struct LoginForm {
    username: String,
    password: String,
}

async fn login_submit(State(state): State<AppState>, Form(f): Form<LoginForm>) -> PageResult {
    let username = auth::normalize_username(&f.username);
    let client = state.pool.get().await.map_err(internal)?;
    let row = client
        .query_opt(
            "SELECT id, password_hash, is_admin, disabled FROM users WHERE username = $1",
            &[&username],
        )
        .await
        .map_err(internal)?;
    let hash = row.as_ref().map(|r| r.get::<_, String>(1));
    let ok = auth::verify_password(f.password, hash).await;
    let user_id: i64 = match row {
        Some(r) if ok && r.get::<_, bool>(2) && !r.get::<_, bool>(3) => r.get(0),
        _ => {
            return Ok(back(
                "/admin/login",
                Err("Invalid credentials or not an admin."),
            ))
        }
    };
    let secret = auth::generate_secret();
    client
        .execute("DELETE FROM admin_sessions WHERE expires_at < now()", &[])
        .await
        .map_err(internal)?;
    client
        .execute(
            "INSERT INTO admin_sessions (id_hash, user_id, expires_at)
             VALUES ($1, $2, now() + make_interval(days => $3::int))",
            &[&secret.hash, &user_id, &(SESSION_DAYS as i32)],
        )
        .await
        .map_err(internal)?;
    let cookie = format!(
        "{COOKIE}={}; HttpOnly; Secure; SameSite=Strict; Path=/admin; Max-Age={}",
        secret.token,
        SESSION_DAYS * 86400
    );
    let mut resp = Redirect::to("/admin").into_response();
    resp.headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    Ok(resp)
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

// ---------- dashboard ----------

async fn dashboard(
    State(state): State<AppState>,
    admin: Admin,
    Query(flash): Query<Flash>,
) -> PageResult {
    let client = state.pool.get().await.map_err(internal)?;
    let rows = client
        .query(
            "SELECT u.id, u.username, u.is_admin, u.disabled,
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
        let yes = |b: bool| if b { "yes" } else { "" };
        table += &format!(
            r#"<tr><td><a href="/admin/users/{id}">{}</a></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
            escape(&name),
            yes(r.get(2)),
            yes(r.get(3)),
            r.get::<_, i64>(4),
            human_bytes(r.get(5)),
            r.get::<_, i64>(6),
            escape(&r.get::<_, String>(7)),
        );
    }
    let body = format!(
        r#"<header><h1>Yomu admin</h1><span>{me} · <a href="/admin/logout">log out</a></span></header>
{flash}
<h2>Users</h2>
<div class="scroll"><table>
<tr><th>Username</th><th>Admin</th><th>Disabled</th><th>Books</th><th>Storage</th><th>Devices</th><th>Created</th></tr>
{table}</table></div>
<fieldset><legend>Create user</legend>
<form method="post" action="/admin/users">
<p><label>Username<br><input type="text" name="username" required autocomplete="off"></label></p>
<p><label>Password<br><input type="password" name="password" required minlength="8" autocomplete="new-password"></label></p>
<p><label><input type="checkbox" name="is_admin" value="on"> Admin</label></p>
<p><button type="submit">Create</button></p>
</form></fieldset>
<p><small>Yomu sync server {ver}</small></p>"#,
        me = escape(&admin.username),
        flash = flash.html(),
        ver = env!("CARGO_PKG_VERSION"),
    );
    Ok(page("Users", &body).into_response())
}

#[derive(Deserialize)]
struct CreateUserForm {
    username: String,
    password: String,
    is_admin: Option<String>,
}

async fn create_user(
    State(state): State<AppState>,
    _admin: Admin,
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
            "INSERT INTO users (username, password_hash, is_admin) VALUES ($1, $2, $3)
             ON CONFLICT (username) DO NOTHING",
            &[&username, &hash, &f.is_admin.is_some()],
        )
        .await
        .map_err(internal)?;
    if n == 0 {
        return Ok(back("/admin", Err("That username is taken.")));
    }
    Ok(back("/admin", Ok(&format!("Created user {username}."))))
}

// ---------- per-user page ----------

struct UserRow {
    username: String,
    is_admin: bool,
    disabled: bool,
}

async fn load_user(
    client: &impl deadpool_postgres::GenericClient,
    id: i64,
) -> Result<Option<UserRow>, Response> {
    Ok(client
        .query_opt(
            "SELECT username, is_admin, disabled FROM users WHERE id = $1",
            &[&id],
        )
        .await
        .map_err(internal)?
        .map(|r| UserRow {
            username: r.get(0),
            is_admin: r.get(1),
            disabled: r.get(2),
        }))
}

async fn user_page(
    State(state): State<AppState>,
    admin: Admin,
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
    let is_self = id == admin.user_id;
    let name = escape(&u.username);
    let body = format!(
        r#"<header><h1>{name}</h1><span><a href="/admin">&larr; all users</a> · <a href="/admin/logout">log out</a></span></header>
{flash}
<p>Admin: <b>{adm}</b> · Disabled: <b>{dis}</b>{you}</p>
<h2>Devices</h2>
<div class="scroll"><table><tr><th>Device</th><th>Signed in (UTC)</th><th>Last used (UTC)</th><th></th></tr>{devices}</table></div>
<fieldset><legend>Reset password</legend>
<form method="post" action="/admin/users/{id}/password">
<p><input type="password" name="password" required minlength="8" autocomplete="new-password" placeholder="new password"></p>
<p><label><input type="checkbox" name="revoke" value="on" checked> Sign out all devices</label></p>
<p><button type="submit">Set password</button></p></form></fieldset>
<fieldset><legend>Status</legend>
<form class="inline" method="post" action="/admin/users/{id}/toggle-disabled"><button type="submit">{dis_btn}</button></form>
<form class="inline" method="post" action="/admin/users/{id}/toggle-admin"><button type="submit">{adm_btn}</button></form>
</fieldset>
<fieldset><legend class="danger">Delete user</legend>
<form method="post" action="/admin/users/{id}/delete">
<p>Deletes the account with all books, settings, translations and files. Type the username to confirm.</p>
<p><input type="text" name="confirm" required autocomplete="off" placeholder="{name}"></p>
<p><button type="submit" class="danger">Delete permanently</button></p></form></fieldset>"#,
        flash = flash.html(),
        adm = if u.is_admin { "yes" } else { "no" },
        dis = if u.disabled { "yes" } else { "no" },
        you = if is_self { " · (this is you)" } else { "" },
        dis_btn = if u.disabled {
            "Enable account"
        } else {
            "Disable account"
        },
        adm_btn = if u.is_admin {
            "Remove admin"
        } else {
            "Make admin"
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

/// Runs `f` inside a transaction holding the admin-change lock, after
/// checking that the change keeps at least one enabled admin.
async fn guarded_change(
    state: &AppState,
    target: i64,
    sql: &str,
    would_remove_enabled_admin: impl Fn(&UserRow) -> bool,
) -> Result<Result<(), &'static str>, Response> {
    let mut client = state.pool.get().await.map_err(internal)?;
    let tx = client.transaction().await.map_err(internal)?;
    tx.execute("SELECT pg_advisory_xact_lock(2, 0)", &[])
        .await
        .map_err(internal)?;
    let Some(u) = load_user(&tx, target).await? else {
        return Ok(Err("No such user."));
    };
    if would_remove_enabled_admin(&u) {
        let admins: i64 = tx
            .query_one(
                "SELECT count(*) FROM users WHERE is_admin AND NOT disabled",
                &[],
            )
            .await
            .map_err(internal)?
            .get(0);
        if admins <= 1 {
            return Ok(Err("Refused: this is the last enabled admin."));
        }
    }
    tx.execute(sql, &[&target]).await.map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Ok(()))
}

async fn toggle_disabled(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<i64>,
) -> PageResult {
    let path = format!("/admin/users/{id}");
    if id == admin.user_id {
        return Ok(back(&path, Err("You cannot disable yourself.")));
    }
    let r = guarded_change(
        &state,
        id,
        "UPDATE users SET disabled = NOT disabled WHERE id = $1",
        |u| u.is_admin && !u.disabled,
    )
    .await?;
    Ok(back(&path, r.map(|_| "Account status changed.")))
}

async fn toggle_admin(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<i64>,
) -> PageResult {
    let path = format!("/admin/users/{id}");
    let r = guarded_change(
        &state,
        id,
        "UPDATE users SET is_admin = NOT is_admin WHERE id = $1",
        |u| u.is_admin && !u.disabled,
    )
    .await?;
    Ok(back(&path, r.map(|_| "Admin role changed.")))
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
    if id == admin.user_id {
        return Ok(back(&path, Err("You cannot delete yourself.")));
    }
    {
        let client = state.pool.get().await.map_err(internal)?;
        match load_user(&client, id).await? {
            None => return Ok(back("/admin", Err("No such user."))),
            Some(u) if u.username != auth::normalize_username(&f.confirm) => {
                return Ok(back(&path, Err("Confirmation did not match the username.")))
            }
            Some(_) => {}
        }
    }
    match guarded_change(&state, id, "DELETE FROM users WHERE id = $1", |u| {
        u.is_admin && !u.disabled
    })
    .await?
    {
        Ok(()) => Ok(back("/admin", Ok("User deleted."))),
        Err(e) => Ok(back(&path, Err(e))),
    }
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
