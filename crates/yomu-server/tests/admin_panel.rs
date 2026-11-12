//! Admin panel sign-in: OIDC against a fake identity provider, the local
//! break-glass login, and app users being kept out.

mod common;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    body::Body,
    extract::{Form, State},
    http::{header, HeaderMap, Method, Request, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;
use yomu_server::{
    app, auth,
    config::{LocalAdmin, OidcConfig},
    oidc::pkce_challenge,
    AppState,
};

const CLIENT_ID: &str = "yomu";
const CLIENT_SECRET: &str = "s3cret:/+";
const REDIRECT: &str = "https://yomu.test/admin/oidc/callback";

#[derive(Default)]
struct Idp {
    issuer: String,
    nonce: String,
    challenge: String,
    groups: Vec<String>,
}

type Shared = Arc<Mutex<Idp>>;

fn unsigned_jwt(claims: serde_json::Value) -> String {
    format!(
        "{}.{}.sig",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256"}"#),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    )
}

async fn discovery(State(idp): State<Shared>) -> impl IntoResponse {
    let iss = idp.lock().unwrap().issuer.clone();
    Json(json!({
        "issuer": iss,
        "authorization_endpoint": format!("{iss}authorize"),
        "token_endpoint": format!("{iss}token"),
        "userinfo_endpoint": format!("{iss}userinfo"),
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
    }))
}

async fn token(
    State(idp): State<Shared>,
    headers: HeaderMap,
    Form(f): Form<HashMap<String, String>>,
) -> axum::response::Response {
    let idp = idp.lock().unwrap();
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    let expected_basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!(
            "{}:{}",
            enc(CLIENT_ID),
            enc(CLIENT_SECRET)
        ))
    );
    let ok = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        == Some(expected_basic.as_str())
        && f.get("grant_type").map(String::as_str) == Some("authorization_code")
        && f.get("code").map(String::as_str) == Some("code-1")
        && f.get("redirect_uri").map(String::as_str) == Some(REDIRECT)
        && f.get("code_verifier").map(|v| pkce_challenge(v)) == Some(idp.challenge.clone());
    if !ok {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant"})),
        )
            .into_response();
    }
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    Json(json!({
        "access_token": "access-1",
        "token_type": "Bearer",
        "id_token": unsigned_jwt(json!({
            "iss": idp.issuer, "aud": CLIENT_ID, "exp": exp, "nonce": idp.nonce, "sub": "sub-1"
        })),
    }))
    .into_response()
}

async fn userinfo(State(idp): State<Shared>, headers: HeaderMap) -> axum::response::Response {
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        != Some("Bearer access-1")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let groups = idp.lock().unwrap().groups.clone();
    Json(json!({"sub": "sub-1", "preferred_username": "boss", "email": "boss@test", "groups": groups}))
        .into_response()
}

/// Starts the fake IdP; returns its issuer URL (with trailing slash, like Authentik).
async fn fake_idp(shared: Shared) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!(
        "http://{}/application/o/yomu/",
        listener.local_addr().unwrap()
    );
    shared.lock().unwrap().issuer = issuer.clone();
    let router = Router::new()
        .route(
            "/application/o/yomu/.well-known/openid-configuration",
            get(discovery),
        )
        .route("/application/o/yomu/token", post(token))
        .route("/application/o/yomu/userinfo", get(userinfo))
        .with_state(shared);
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    issuer
}

struct Resp {
    status: StatusCode,
    location: String,
    cookies: Vec<String>,
    body: String,
}

async fn send(app: &Router, method: Method, uri: &str, cookie: &str, form: &str) -> Resp {
    let mut req = Request::builder().method(method).uri(uri);
    if !cookie.is_empty() {
        req = req.header(header::COOKIE, cookie);
    }
    if !form.is_empty() {
        req = req.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    }
    let resp = app
        .clone()
        .oneshot(req.body(Body::from(form.to_string())).unwrap())
        .await
        .unwrap();
    let location = resp
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let cookies = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    let status = resp.status();
    let body =
        String::from_utf8_lossy(&resp.into_body().collect().await.unwrap().to_bytes()).into();
    Resp {
        status,
        location,
        cookies,
        body,
    }
}

/// `name=value` of a Set-Cookie header with that name.
fn cookie_pair(r: &Resp, name: &str) -> Option<String> {
    r.cookies
        .iter()
        .find(|c| c.starts_with(&format!("{name}=")))
        .map(|c| c.split(';').next().unwrap().to_string())
}

fn oidc_config(issuer: &str, group: Option<&str>) -> OidcConfig {
    OidcConfig {
        issuer: issuer.into(),
        client_id: CLIENT_ID.into(),
        client_secret: CLIENT_SECRET.into(),
        redirect_uri: REDIRECT.into(),
        admin_group: group.map(String::from),
    }
}

/// Runs /admin/oidc/start and hands the auth request parameters to the IdP.
async fn start(app: &Router, idp: &Shared) -> (String, String) {
    let r = send(app, Method::GET, "/admin/oidc/start", "", "").await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    let issuer = idp.lock().unwrap().issuer.clone();
    assert!(r.location.starts_with(&format!("{issuer}authorize?")));
    let cookie = r
        .cookies
        .iter()
        .find(|c| c.starts_with("yomu_oidc="))
        .unwrap();
    for attr in [
        "HttpOnly",
        "Secure",
        "SameSite=Lax",
        "Path=/admin/oidc",
        "Max-Age=600",
    ] {
        assert!(cookie.contains(attr), "{cookie} lacks {attr}");
    }
    let q: HashMap<String, String> = url::Url::parse(&r.location)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    assert_eq!(q["client_id"], CLIENT_ID);
    assert_eq!(q["redirect_uri"], REDIRECT);
    assert_eq!(q["scope"], "openid profile email");
    assert_eq!(q["code_challenge_method"], "S256");
    {
        let mut i = idp.lock().unwrap();
        i.nonce = q["nonce"].clone();
        i.challenge = q["code_challenge"].clone();
    }
    (cookie_pair(&r, "yomu_oidc").unwrap(), q["state"].clone())
}

#[tokio::test]
async fn admin_panel_sign_in() {
    let Some((base_config, pool)) = common::fresh_db(None).await else {
        eprintln!("YOMU_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let idp: Shared = Arc::default();
    let issuer = fake_idp(idp.clone()).await;

    let mut config = base_config.clone();
    config.oidc = Some(oidc_config(&issuer, None));
    config.local_admin = Some(LocalAdmin {
        username: "root".into(),
        password: "break-glass-pw".into(),
    });
    let app = app(AppState::new(pool.clone(), config));

    // Login page offers both methods.
    let r = send(&app, Method::GET, "/admin/login", "", "").await;
    assert!(r.body.contains("Sign in with SSO") && r.body.contains("name=\"password\""));
    // Not signed in → redirected.
    let r = send(&app, Method::GET, "/admin", "", "").await;
    assert_eq!(r.location, "/admin/login");

    // --- OIDC happy path ---
    let (oidc_cookie, state) = start(&app, &idp).await;
    let r = send(
        &app,
        Method::GET,
        &format!("/admin/oidc/callback?code=code-1&state={state}"),
        &oidc_cookie,
        "",
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    assert_eq!(r.location, "/admin");
    assert!(r.cookies.iter().any(|c| c.starts_with("yomu_oidc=;")));
    let session = cookie_pair(&r, "yomu_admin").expect("session cookie");
    let r = send(&app, Method::GET, "/admin", &session, "").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("oidc:sub-1 (boss)"), "{}", r.body);

    // --- OIDC failures ---
    let (oidc_cookie, _state) = start(&app, &idp).await;
    let r = send(
        &app,
        Method::GET,
        "/admin/oidc/callback?code=code-1&state=forged",
        &oidc_cookie,
        "",
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(cookie_pair(&r, "yomu_admin").is_none());
    let r = send(
        &app,
        Method::GET,
        "/admin/oidc/callback?code=code-1&state=x",
        "",
        "",
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "no state cookie");
    // Replayed state with a different nonce at the IdP → id_token nonce mismatch.
    let (oidc_cookie, state) = start(&app, &idp).await;
    idp.lock().unwrap().nonce = "other-nonce".into();
    let r = send(
        &app,
        Method::GET,
        &format!("/admin/oidc/callback?code=code-1&state={state}"),
        &oidc_cookie,
        "",
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_GATEWAY);
    assert!(r.body.contains("nonce"), "{}", r.body);
    let r = send(
        &app,
        Method::GET,
        "/admin/oidc/callback?error=access_denied&error_description=policy",
        "",
        "",
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    // Admin group required but missing → 403; present → OK.
    let mut grouped = base_config.clone();
    grouped.oidc = Some(oidc_config(&issuer, Some("yomu-admins")));
    let gapp = yomu_server::app(AppState::new(pool.clone(), grouped));
    let (oidc_cookie, state) = start(&gapp, &idp).await;
    let cb = format!("/admin/oidc/callback?code=code-1&state={state}");
    let r = send(&gapp, Method::GET, &cb, &oidc_cookie, "").await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    idp.lock().unwrap().groups = vec!["yomu-admins".into()];
    let (oidc_cookie, state) = start(&gapp, &idp).await;
    let cb = format!("/admin/oidc/callback?code=code-1&state={state}");
    let r = send(&gapp, Method::GET, &cb, &oidc_cookie, "").await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    // That app has no local admin: the password form is gone and posts fail.
    let r = send(&gapp, Method::GET, "/admin/login", "", "").await;
    assert!(!r.body.contains("name=\"password\""));

    // Unreachable IdP → fail closed with an error page.
    let mut down = base_config.clone();
    down.oidc = Some(oidc_config("http://127.0.0.1:9/nope/", None));
    let dapp = yomu_server::app(AppState::new(pool.clone(), down));
    let r = send(&dapp, Method::GET, "/admin/oidc/start", "", "").await;
    assert_eq!(r.status, StatusCode::BAD_GATEWAY);
    assert!(r.location.is_empty());

    // --- local break-glass admin ---
    let r = send(
        &app,
        Method::POST,
        "/admin/login",
        "",
        "username=ROOT&password=break-glass-pw",
    )
    .await;
    assert_eq!(r.location, "/admin");
    let local = cookie_pair(&r, "yomu_admin").unwrap();
    let r = send(&app, Method::GET, "/admin", &local, "").await;
    assert!(r.body.contains("local:root"));
    let r = send(
        &app,
        Method::POST,
        "/admin/login",
        "",
        "username=root&password=wrong",
    )
    .await;
    assert!(r.location.starts_with("/admin/login?err="));

    // The admin manages app users; app users cannot sign in to the panel.
    let r = send(
        &app,
        Method::POST,
        "/admin/users",
        &local,
        "username=reader&password=reader-pw-1",
    )
    .await;
    assert!(r.location.contains("msg="), "{}", r.location);
    let hash: String = pool
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT password_hash FROM users WHERE username = 'reader'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(auth::verify_password("reader-pw-1".into(), Some(hash)).await);
    let r = send(
        &app,
        Method::POST,
        "/admin/login",
        "",
        "username=reader&password=reader-pw-1",
    )
    .await;
    assert!(r.location.starts_with("/admin/login?err="));
    assert!(cookie_pair(&r, "yomu_admin").is_none());

    // Disabling the local method ends its sessions; OIDC sessions survive.
    let mut no_local = base_config.clone();
    no_local.oidc = Some(oidc_config(&issuer, None));
    let napp = yomu_server::app(AppState::new(pool.clone(), no_local));
    let r = send(&napp, Method::GET, "/admin", &local, "").await;
    assert_eq!(r.location, "/admin/login");
    let r = send(&napp, Method::GET, "/admin", &session, "").await;
    assert_eq!(r.status, StatusCode::OK);

    // Logout.
    let r = send(&app, Method::POST, "/admin/logout", &session, "").await;
    assert!(r.location.starts_with("/admin/login"));
    let r = send(&app, Method::GET, "/admin", &session, "").await;
    assert_eq!(r.location, "/admin/login");

    // No method configured → panel says it's disabled.
    let papp = yomu_server::app(AppState::new(pool.clone(), base_config));
    let r = send(&papp, Method::GET, "/admin/login", "", "").await;
    assert!(r.body.contains("admin panel is disabled"));
}
