//! End-to-end API test against a real Postgres. Skipped unless
//! `YOMU_TEST_DATABASE_URL` is set, e.g.
//! `YOMU_TEST_DATABASE_URL=postgres://postgres:test@localhost:55432/postgres`.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use yomu_server::{app, auth, db, AppState, Config};

async fn setup() -> Option<(Router, deadpool_postgres::Pool)> {
    let url = std::env::var("YOMU_TEST_DATABASE_URL").ok()?;
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let config = Config::for_database(url);
    let pool = db::make_pool(&config).unwrap();
    db::wait_for_db(&pool, std::time::Duration::from_secs(10))
        .await
        .unwrap();
    db::migrate(&pool).await.unwrap();
    let state = AppState {
        pool: pool.clone(),
        config: Arc::new(config),
    };
    Some((app(state), pool))
}

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Body,
    content_type: &str,
) -> (StatusCode, Vec<u8>) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    req = req.header(header::CONTENT_TYPE, content_type);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, bytes)
}

async fn json_call(
    app: &Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let (s, b) = call(
        app,
        method,
        uri,
        token,
        Body::from(body.to_string()),
        "application/json",
    )
    .await;
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

async fn create_user(pool: &deadpool_postgres::Pool, name: &str, pw: &str) {
    let hash = auth::hash_password(pw.into()).await;
    pool.get()
        .await
        .unwrap()
        .execute(
            "INSERT INTO users (username, password_hash) VALUES ($1, $2)",
            &[&name, &hash],
        )
        .await
        .unwrap();
}

async fn login(app: &Router, user: &str, pw: &str, device: &str) -> String {
    let (s, v) = json_call(
        app,
        Method::POST,
        "/api/v1/login",
        None,
        json!({"username": user, "password": pw, "device_name": device}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["token"].as_str().unwrap().to_string()
}

async fn sync(app: &Router, token: &str, body: Value) -> Value {
    let (s, v) = json_call(app, Method::POST, "/api/v1/sync", Some(token), body).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

#[tokio::test]
async fn login_sync_and_files_roundtrip() {
    let Some((app, pool)) = setup().await else {
        eprintln!("YOMU_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let suffix = hex::encode(rand::random::<[u8; 4]>());
    let user = format!("tester{suffix}");
    create_user(&pool, &user, "password123").await;

    // Bad credentials and unknown users look the same.
    for (u, p) in [(user.as_str(), "wrong"), ("nobody-here", "password123")] {
        let (s, v) = json_call(
            &app,
            Method::POST,
            "/api/v1/login",
            None,
            json!({"username": u, "password": p, "device_name": "x"}),
        )
        .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        assert!(v["error"].is_string());
    }

    // Username is case-insensitive.
    let a = login(&app, &user.to_uppercase(), "password123", "laptop").await;
    let b = login(&app, &user, "password123", "phone").await;
    let (s, v) = json_call(&app, Method::GET, "/api/v1/me", Some(&a), Value::Null).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v, json!({"username": user, "is_admin": false}));
    let (s, _) = json_call(&app, Method::GET, "/api/v1/me", Some("nope"), Value::Null).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // Device A pushes.
    let book = |pos: i64, updated_at: i64| {
        json!({"key": "book-1", "title": "本", "author": "著者", "position": pos,
               "last_read_at": 1700000000000i64, "added_at": 1600000000000i64,
               "deleted": false, "updated_at": updated_at})
    };
    let pushed = sync(
        &app,
        &a,
        json!({
            "since": 0,
            "books": [book(10, 1000)],
            "settings": {"data": {"theme": "dark"}, "updated_at": 1000},
            "translations": [{"key": "book-1", "idx": 3, "lang": "English",
                              "text": "{\"t\":\"hello\"}", "model": "m", "updated_at": 1000}]
        }),
    )
    .await;
    assert!(pushed["cursor"].as_i64().unwrap() > 0);
    assert_eq!(pushed["more"], json!(false));

    // Device B pulls everything.
    let pulled = sync(&app, &b, json!({"since": 0})).await;
    assert_eq!(pulled["books"], json!([book(10, 1000)]));
    assert_eq!(
        pulled["settings"],
        json!({"data": {"theme": "dark"}, "updated_at": 1000})
    );
    assert_eq!(pulled["translations"][0]["text"], "{\"t\":\"hello\"}");
    assert_eq!(pulled["files"], json!([]));
    let cursor_b = pulled["cursor"].as_i64().unwrap();

    // B pushes a stale update: rejected, nothing new to pull.
    let stale = sync(
        &app,
        &b,
        json!({"since": cursor_b, "books": [book(99, 500)], "settings": {"data": {}, "updated_at": 1000}}),
    )
    .await;
    assert_eq!(stale["books"], json!([]));
    assert_eq!(stale["settings"], Value::Null);
    assert_eq!(stale["cursor"].as_i64().unwrap(), cursor_b);

    // A pushes a newer position; B sees exactly that.
    sync(&app, &a, json!({"since": 0, "books": [book(42, 2000)]})).await;
    let newer = sync(&app, &b, json!({"since": cursor_b})).await;
    assert_eq!(newer["books"], json!([book(42, 2000)]));
    assert_eq!(newer["translations"], json!([]));

    // Pagination: more than one page of translations.
    let many: Vec<Value> = (0..2500)
        .map(|i| json!({"key": "book-1", "idx": 1000 + i, "lang": "English", "text": "x", "model": "", "updated_at": 5}))
        .collect();
    sync(&app, &a, json!({"since": 0, "translations": many})).await;
    let mut since = 0;
    let mut total = 0;
    let mut pages = 0;
    loop {
        let page = sync(&app, &b, json!({"since": since})).await;
        total += page["translations"].as_array().unwrap().len();
        since = page["cursor"].as_i64().unwrap();
        pages += 1;
        if page["more"] == json!(false) {
            break;
        }
    }
    assert_eq!(total, 2501);
    assert!(pages >= 2);

    // Files.
    let epub = b"PK\x03\x04 fake epub bytes".to_vec();
    let (s, _) = call(
        &app,
        Method::PUT,
        "/api/v1/files/book-1",
        Some(&a),
        Body::from(epub.clone()),
        "application/epub+zip",
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, got) = call(
        &app,
        Method::GET,
        "/api/v1/files/book-1",
        Some(&b),
        Body::empty(),
        "",
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(got, epub);
    let (s, _) = call(
        &app,
        Method::GET,
        "/api/v1/files/missing",
        Some(&b),
        Body::empty(),
        "",
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let listed = sync(&app, &b, json!({"since": since})).await;
    assert_eq!(listed["files"], json!(["book-1"]));

    // Logout revokes only that token.
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/v1/logout",
        Some(&a),
        Body::empty(),
        "",
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = json_call(&app, Method::GET, "/api/v1/me", Some(&a), Value::Null).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = json_call(&app, Method::GET, "/api/v1/me", Some(&b), Value::Null).await;
    assert_eq!(s, StatusCode::OK);

    // Cleanup.
    pool.get()
        .await
        .unwrap()
        .execute("DELETE FROM users WHERE username = $1", &[&user])
        .await
        .unwrap();
}
