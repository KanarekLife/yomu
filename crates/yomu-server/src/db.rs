//! Connection pool, embedded migrations and admin bootstrap.

use std::{sync::Arc, time::Duration};

use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    DigitallySignedStruct, SignatureScheme,
};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::{auth, config::Config};

/// Embedded SQL migrations, applied in order. Never edit an applied one;
/// append a new file instead.
const MIGRATIONS: &[(i32, &str, &str)] =
    &[(1, "init", include_str!("../migrations/0001_init.sql"))];

/// Builds the pool. TLS follows libpq's `sslmode` semantics from
/// `DATABASE_URL` (`disable` / `prefer` (default) / `require`): the channel is
/// encrypted when the server offers TLS, but the certificate is not verified
/// (libpq's `require` behaves the same way). In-cluster Postgres typically
/// uses a self-signed CA.
pub fn make_pool(config: &Config) -> Result<Pool, String> {
    let pg: tokio_postgres::Config = config
        .database_url
        .parse()
        .map_err(|e| format!("invalid DATABASE_URL: {e}"))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let tls_config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
        .with_no_client_auth();
    let tls = MakeRustlsConnect::new(tls_config);
    let mgr = Manager::from_config(
        pg,
        tls,
        ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        },
    );
    Pool::builder(mgr)
        .max_size(16)
        .wait_timeout(Some(Duration::from_secs(30)))
        .create_timeout(Some(Duration::from_secs(15)))
        .runtime(deadpool_postgres::Runtime::Tokio1)
        .build()
        .map_err(|e| e.to_string())
}

/// Waits (up to `max_wait`) until the database accepts connections; a cold
/// cluster may take minutes to bootstrap Postgres.
pub async fn wait_for_db(pool: &Pool, max_wait: Duration) -> Result<(), String> {
    let start = std::time::Instant::now();
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let err = match pool.get().await {
            Ok(c) => match c.simple_query("SELECT 1").await {
                Ok(_) => return Ok(()),
                Err(e) => e.to_string(),
            },
            Err(e) => e.to_string(),
        };
        if start.elapsed() >= max_wait {
            return Err(format!(
                "database not reachable after {attempt} attempts: {err}"
            ));
        }
        tracing::warn!(attempt, error = %err, "database not ready, retrying in 5s");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Applies pending migrations in one transaction, serialised across replicas
/// by an advisory lock.
pub async fn migrate(pool: &Pool) -> Result<(), String> {
    let mut client = pool.get().await.map_err(|e| e.to_string())?;
    let tx = client.transaction().await.map_err(|e| e.to_string())?;
    tx.execute("SELECT pg_advisory_xact_lock(7340987001)", &[])
        .await
        .map_err(|e| e.to_string())?;
    tx.batch_execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INT PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT now())",
    )
    .await
    .map_err(|e| e.to_string())?;
    for (version, name, sql) in MIGRATIONS {
        let done = tx
            .query_opt(
                "SELECT 1 FROM schema_migrations WHERE version = $1",
                &[version],
            )
            .await
            .map_err(|e| e.to_string())?
            .is_some();
        if done {
            continue;
        }
        tracing::info!(version, name, "applying migration");
        tx.batch_execute(sql)
            .await
            .map_err(|e| format!("migration {version} ({name}) failed: {e}"))?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name) VALUES ($1, $2)",
            &[version, name],
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())
}

/// Creates the configured admin when the server has no admin at all. If a
/// user with that name already exists it is promoted, re-enabled and gets the
/// configured password (a recovery path for a lost admin).
pub async fn ensure_admin(pool: &Pool, config: &Config) -> Result<(), String> {
    let client = pool.get().await.map_err(|e| e.to_string())?;
    let has_admin = client
        .query_one("SELECT EXISTS (SELECT 1 FROM users WHERE is_admin)", &[])
        .await
        .map_err(|e| e.to_string())?
        .get::<_, bool>(0);
    if has_admin {
        return Ok(());
    }
    let (Some(user), Some(password)) = (&config.admin_user, &config.admin_password) else {
        tracing::warn!(
            "no admin user exists; set YOMU_ADMIN_USER and YOMU_ADMIN_PASSWORD to create one"
        );
        return Ok(());
    };
    let username = auth::normalize_username(user);
    auth::validate_username(&username).map_err(|e| format!("YOMU_ADMIN_USER: {e}"))?;
    let hash = auth::hash_password(password.clone()).await;
    client
        .execute(
            "INSERT INTO users (username, password_hash, is_admin) VALUES ($1, $2, TRUE)
             ON CONFLICT (username) DO UPDATE
               SET password_hash = EXCLUDED.password_hash, is_admin = TRUE, disabled = FALSE",
            &[&username, &hash],
        )
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(username, "created admin user");
    Ok(())
}

/// Accepts any server certificate (libpq `sslmode=require` semantics) while
/// still checking handshake signatures.
#[derive(Debug)]
struct NoVerify(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
