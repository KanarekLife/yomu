//! Upgrade from the 0001 schema (admins were app users) to the current one.

mod common;

use yomu_server::db;

#[tokio::test]
async fn upgrade_drops_empty_admins_and_keeps_data() {
    let Some((_config, pool)) = common::fresh_db(Some(1)).await else {
        eprintln!("YOMU_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let c = pool.get().await.unwrap();
    c.batch_execute(
        "INSERT INTO users (username, password_hash, is_admin) VALUES
           ('admin', 'x', TRUE),          -- bootstrap leftover: owns nothing
           ('reader-admin', 'x', TRUE),   -- admin who also used the app
           ('settings-admin', 'x', TRUE), -- owns only settings
           ('plain', 'x', FALSE);         -- ordinary user without data
         INSERT INTO books (user_id, key, updated_at, rev)
           SELECT id, 'b1', 1, nextval('rev_seq') FROM users WHERE username = 'reader-admin';
         INSERT INTO settings (user_id, data, updated_at, rev)
           SELECT id, '{}', 1, nextval('rev_seq') FROM users WHERE username = 'settings-admin';
         INSERT INTO tokens (token_hash, user_id)
           SELECT '\\x01', id FROM users WHERE username = 'admin';
         INSERT INTO admin_sessions (id_hash, user_id, expires_at)
           SELECT '\\x02', id, now() + interval '1 day' FROM users WHERE username = 'admin';",
    )
    .await
    .unwrap();
    drop(c);

    db::migrate(&pool).await.unwrap();
    // Re-running is a no-op.
    db::migrate(&pool).await.unwrap();

    let c = pool.get().await.unwrap();
    let names: Vec<String> = c
        .query("SELECT username FROM users ORDER BY username", &[])
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(names, ["plain", "reader-admin", "settings-admin"]);

    let has_is_admin: bool = c
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_schema = current_schema() AND table_name = 'users'
                 AND column_name = 'is_admin')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!has_is_admin);

    let books: i64 = c
        .query_one("SELECT count(*) FROM books", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(books, 1);
    let tokens: i64 = c
        .query_one("SELECT count(*) FROM tokens", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(tokens, 0, "the deleted admin's tokens cascade");
    let sessions: i64 = c
        .query_one("SELECT count(*) FROM admin_sessions", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(sessions, 0);
    c.execute(
        "INSERT INTO admin_sessions (id_hash, admin, expires_at) VALUES ('\\x03', 'local:root', now())",
        &[],
    )
    .await
    .unwrap();
}
