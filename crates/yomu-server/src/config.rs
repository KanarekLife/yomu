use std::net::SocketAddr;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub listen: SocketAddr,
    pub admin_user: Option<String>,
    pub admin_password: Option<String>,
    pub max_file_bytes: usize,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required".to_string())?;
        let listen = std::env::var("YOMU_LISTEN")
            .unwrap_or_else(|_| "0.0.0.0:8080".into())
            .parse()
            .map_err(|e| format!("invalid YOMU_LISTEN: {e}"))?;
        let max_file_mb: usize = match std::env::var("YOMU_MAX_FILE_MB") {
            Ok(v) => v
                .trim()
                .parse()
                .map_err(|e| format!("invalid YOMU_MAX_FILE_MB: {e}"))?,
            Err(_) => 200,
        };
        let nonempty = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        Ok(Config {
            database_url,
            listen,
            admin_user: nonempty("YOMU_ADMIN_USER"),
            admin_password: nonempty("YOMU_ADMIN_PASSWORD"),
            max_file_bytes: max_file_mb * 1024 * 1024,
        })
    }

    /// Config for tests / embedding.
    pub fn for_database(database_url: impl Into<String>) -> Self {
        Config {
            database_url: database_url.into(),
            listen: "127.0.0.1:0".parse().unwrap(),
            admin_user: None,
            admin_password: None,
            max_file_bytes: 200 * 1024 * 1024,
        }
    }
}
