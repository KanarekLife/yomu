use std::net::SocketAddr;

/// Admin-panel sign-in via an OpenID Connect provider.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    /// `YOMU_PUBLIC_URL` + `/admin/oidc/callback`.
    pub redirect_uri: String,
    /// If set, the userinfo `groups` claim must contain it.
    pub admin_group: Option<String>,
}

/// Break-glass panel login from the environment (never a row in `users`).
#[derive(Clone)]
pub struct LocalAdmin {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for LocalAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalAdmin")
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub listen: SocketAddr,
    pub max_file_bytes: usize,
    pub oidc: Option<OidcConfig>,
    pub local_admin: Option<LocalAdmin>,
}

fn parse_bool(key: &str, v: &str) -> Result<bool, String> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("invalid {key}: expected true or false")),
    }
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Builds the config from any key → value lookup (tests use a map).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let var = |k: &str| get(k).filter(|v| !v.trim().is_empty());
        let database_url = var("DATABASE_URL").ok_or("DATABASE_URL is required")?;
        let listen = var("YOMU_LISTEN")
            .unwrap_or_else(|| "0.0.0.0:8080".into())
            .parse()
            .map_err(|e| format!("invalid YOMU_LISTEN: {e}"))?;
        let max_file_mb: usize = match var("YOMU_MAX_FILE_MB") {
            Some(v) => v
                .trim()
                .parse()
                .map_err(|e| format!("invalid YOMU_MAX_FILE_MB: {e}"))?,
            None => 200,
        };

        let oidc = match (
            var("YOMU_OIDC_ISSUER"),
            var("YOMU_OIDC_CLIENT_ID"),
            var("YOMU_OIDC_CLIENT_SECRET"),
        ) {
            (Some(issuer), Some(client_id), Some(client_secret)) => {
                let public = var("YOMU_PUBLIC_URL")
                    .ok_or("YOMU_PUBLIC_URL is required when OIDC is configured")?;
                let public = public.trim().trim_end_matches('/');
                if !(public.starts_with("https://") || public.starts_with("http://")) {
                    return Err("YOMU_PUBLIC_URL must start with https:// or http://".into());
                }
                Some(OidcConfig {
                    issuer: issuer.trim().to_string(),
                    client_id: client_id.trim().to_string(),
                    client_secret,
                    redirect_uri: format!("{public}/admin/oidc/callback"),
                    admin_group: var("YOMU_OIDC_ADMIN_GROUP").map(|g| g.trim().to_string()),
                })
            }
            (None, None, None) => None,
            _ => {
                return Err("set all of YOMU_OIDC_ISSUER, YOMU_OIDC_CLIENT_ID and \
                     YOMU_OIDC_CLIENT_SECRET (or none of them)"
                    .into())
            }
        };

        let password = var("YOMU_ADMIN_PASSWORD");
        let local_enabled = match var("YOMU_ADMIN_LOCAL") {
            Some(v) => parse_bool("YOMU_ADMIN_LOCAL", &v)?,
            None => password.is_some(),
        };
        let local_admin = match (local_enabled, var("YOMU_ADMIN_USER"), password) {
            (false, _, _) => None,
            (true, Some(user), Some(password)) => Some(LocalAdmin {
                username: user.trim().to_lowercase(),
                password,
            }),
            (true, _, _) => {
                return Err(
                    "local admin login needs YOMU_ADMIN_USER and YOMU_ADMIN_PASSWORD \
                     (or set YOMU_ADMIN_LOCAL=false)"
                        .into(),
                )
            }
        };

        Ok(Config {
            database_url,
            listen,
            max_file_bytes: max_file_mb * 1024 * 1024,
            oidc,
            local_admin,
        })
    }

    /// Config for tests / embedding: no admin login methods.
    pub fn for_database(database_url: impl Into<String>) -> Self {
        Config {
            database_url: database_url.into(),
            listen: "127.0.0.1:0".parse().unwrap(),
            max_file_bytes: 200 * 1024 * 1024,
            oidc: None,
            local_admin: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg(pairs: &[(&str, &str)]) -> Result<Config, String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(|k| m.get(k).cloned())
    }

    #[test]
    fn local_admin_defaults_on_with_password() {
        let c = cfg(&[
            ("DATABASE_URL", "postgres://x"),
            ("YOMU_ADMIN_USER", "Root"),
            ("YOMU_ADMIN_PASSWORD", "pw"),
        ])
        .unwrap();
        assert_eq!(c.local_admin.unwrap().username, "root");
        assert!(c.oidc.is_none());

        let c = cfg(&[
            ("DATABASE_URL", "postgres://x"),
            ("YOMU_ADMIN_USER", "root"),
            ("YOMU_ADMIN_PASSWORD", "pw"),
            ("YOMU_ADMIN_LOCAL", "false"),
        ])
        .unwrap();
        assert!(c.local_admin.is_none());

        let c = cfg(&[("DATABASE_URL", "postgres://x")]).unwrap();
        assert!(c.local_admin.is_none() && c.oidc.is_none());

        assert!(cfg(&[("DATABASE_URL", "x"), ("YOMU_ADMIN_LOCAL", "true")]).is_err());
    }

    #[test]
    fn oidc_needs_all_parts_and_public_url() {
        let base = [
            ("DATABASE_URL", "postgres://x"),
            (
                "YOMU_OIDC_ISSUER",
                "https://login.example/application/o/yomu/",
            ),
            ("YOMU_OIDC_CLIENT_ID", "id"),
            ("YOMU_OIDC_CLIENT_SECRET", "secret"),
        ];
        assert!(cfg(&base).is_err(), "public url missing");
        let mut with_url = base.to_vec();
        with_url.push(("YOMU_PUBLIC_URL", "https://yomu.example/"));
        let o = cfg(&with_url).unwrap().oidc.unwrap();
        assert_eq!(o.redirect_uri, "https://yomu.example/admin/oidc/callback");
        assert!(o.admin_group.is_none());
        assert!(cfg(&base[..3]).is_err(), "partial OIDC config");
    }
}
