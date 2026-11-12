//! Passwords, bearer tokens and the authenticated-user extractor.

use std::sync::OnceLock;

use argon2::{
    password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
    Argon2,
};
use axum::{extract::FromRequestParts, http::request::Parts};
use sha2::{Digest, Sha256};

use crate::{error::ApiError, AppState};

/// A freshly generated secret: the value handed to the client and the
/// SHA-256 digest that is stored server-side.
pub struct NewSecret {
    pub token: String,
    pub hash: Vec<u8>,
}

/// 32 random bytes, hex encoded for the client; only the hash is stored.
pub fn generate_secret() -> NewSecret {
    let bytes: [u8; 32] = rand::random();
    let token = hex::encode(bytes);
    let hash = hash_secret(&token);
    NewSecret { token, hash }
}

/// SHA-256 of the token string as given to the client.
pub fn hash_secret(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Usernames are case-insensitive; stored lowercase and trimmed.
pub fn normalize_username(u: &str) -> String {
    u.trim().to_lowercase()
}

pub fn validate_username(u: &str) -> Result<(), &'static str> {
    if u.is_empty() || u.chars().count() > 64 {
        return Err("username must be 1-64 characters");
    }
    if !u
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '@'))
    {
        return Err("username may only contain letters, digits, '.', '_', '-' and '@'");
    }
    Ok(())
}

pub fn validate_password(p: &str) -> Result<(), &'static str> {
    if p.chars().count() < 8 {
        return Err("password must be at least 8 characters");
    }
    if p.len() > 1024 {
        return Err("password is too long");
    }
    Ok(())
}

fn hash_password_sync(password: &str) -> String {
    Argon2::default()
        .hash_password(password.as_bytes())
        .expect("argon2 hashing failed")
        .to_string()
}

fn verify_password_sync(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Argon2 is deliberately slow; keep it off the async worker threads.
pub async fn hash_password(password: String) -> String {
    tokio::task::spawn_blocking(move || hash_password_sync(&password))
        .await
        .expect("hash task panicked")
}

/// Verifies `password` against `hash`. When `hash` is `None` (unknown user) a
/// dummy hash is checked so the response time doesn't reveal whether the
/// username exists.
pub async fn verify_password(password: String, hash: Option<String>) -> bool {
    tokio::task::spawn_blocking(move || match hash {
        Some(h) => verify_password_sync(&password, &h),
        None => {
            static DUMMY: OnceLock<String> = OnceLock::new();
            let dummy = DUMMY.get_or_init(|| hash_password_sync("yomu-dummy-password"));
            let _ = verify_password_sync(&password, dummy);
            false
        }
    })
    .await
    .unwrap_or(false)
}

/// A request authenticated with a valid bearer token of an enabled user.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: i64,
    pub username: String,
    pub is_admin: bool,
    pub token_hash: Vec<u8>,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| ApiError::unauthorized("missing bearer token"))?;
        let token = header
            .strip_prefix("Bearer ")
            .or_else(|| header.strip_prefix("bearer "))
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| ApiError::unauthorized("missing bearer token"))?;
        let token_hash = hash_secret(token);
        let client = state.db().await?;
        let row = client
            .query_opt(
                "UPDATE tokens t SET last_used_at = now()
                 FROM users u
                 WHERE t.token_hash = $1 AND u.id = t.user_id AND NOT u.disabled
                 RETURNING u.id, u.username, u.is_admin",
                &[&token_hash],
            )
            .await?
            .ok_or_else(|| ApiError::unauthorized("invalid or revoked token"))?;
        Ok(AuthUser {
            user_id: row.get(0),
            username: row.get(1),
            is_admin: row.get(2),
            token_hash,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_random_hex_and_hash_matches() {
        let a = generate_secret();
        let b = generate_secret();
        assert_eq!(a.token.len(), 64);
        assert!(a.token.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a.token, b.token);
        assert_eq!(a.hash.len(), 32);
        assert_eq!(a.hash, hash_secret(&a.token));
        assert_ne!(a.hash, b.hash);
    }

    #[test]
    fn hash_is_sha256_of_token_string() {
        // sha256("abc")
        assert_eq!(
            hex::encode(hash_secret("abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn password_roundtrip() {
        let h = hash_password_sync("correct horse");
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password_sync("correct horse", &h));
        assert!(!verify_password_sync("wrong horse", &h));
        assert!(!verify_password_sync("x", "not a hash"));
    }

    #[test]
    fn usernames_normalize() {
        assert_eq!(normalize_username("  Alice "), "alice");
        assert!(validate_username("alice").is_ok());
        assert!(validate_username("").is_err());
        assert!(validate_username("a b").is_err());
        assert!(validate_username("bob<script>").is_err());
        assert!(validate_username("jan.k-1@home").is_ok());
    }
}
