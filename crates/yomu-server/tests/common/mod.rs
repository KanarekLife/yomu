//! Shared setup for DB-backed tests (skipped unless `YOMU_TEST_DATABASE_URL`
//! is set). Each test gets its own fresh schema so tests don't interfere.

#![allow(dead_code)]

use std::time::Duration;

use deadpool_postgres::Pool;
use yomu_server::{db, Config};

/// Creates a new schema, returns a config/pool scoped to it with migrations
/// applied up to `max_version` (all when `None`).
pub async fn fresh_db(max_version: Option<i32>) -> Option<(Config, Pool)> {
    let url = std::env::var("YOMU_TEST_DATABASE_URL").ok()?;
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let schema = format!("t_{}", hex::encode(rand::random::<[u8; 6]>()));

    let base = db::make_pool(&Config::for_database(url.clone())).unwrap();
    db::wait_for_db(&base, Duration::from_secs(10))
        .await
        .unwrap();
    base.get()
        .await
        .unwrap()
        .batch_execute(&format!("CREATE SCHEMA {schema}"))
        .await
        .unwrap();

    let sep = if url.contains('?') { '&' } else { '?' };
    let scoped = format!("{url}{sep}options=-c%20search_path%3D{schema}");
    let config = Config::for_database(scoped);
    let pool = db::make_pool(&config).unwrap();
    db::migrate_to(&pool, max_version.unwrap_or(i32::MAX))
        .await
        .unwrap();
    Some((config, pool))
}
