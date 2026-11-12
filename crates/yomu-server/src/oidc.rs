//! OpenID Connect sign-in for the admin panel: authorization code flow with
//! PKCE (S256), confidential client.
//!
//! The id_token is received directly from the token endpoint over TLS, so per
//! OIDC Core 3.1.3.7 its signature is not verified; its `iss`, `aud`, `exp`
//! and `nonce` claims are checked. Identity details come from userinfo.

use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

use crate::{auth::secret_eq, config::OidcConfig};

pub const SCOPES: &str = "openid profile email";
/// Discovery documents are re-fetched after this long.
const DISCOVERY_TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Option<Vec<String>>,
}

impl Discovery {
    /// client_secret_basic unless the provider only lists client_secret_post
    /// (an absent list means basic, the spec default).
    pub fn use_basic_auth(&self) -> bool {
        match &self.token_endpoint_auth_methods_supported {
            None => true,
            Some(m) => {
                m.iter().any(|x| x == "client_secret_basic")
                    || !m.iter().any(|x| x == "client_secret_post")
            }
        }
    }
}

/// Per-attempt secrets, kept in a short-lived cookie between the redirect to
/// the IdP and the callback.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthRequest {
    pub state: String,
    pub nonce: String,
    pub verifier: String,
}

fn random_b64() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

impl AuthRequest {
    pub fn generate() -> Self {
        AuthRequest {
            state: random_b64(),
            nonce: random_b64(),
            verifier: random_b64(),
        }
    }

    pub fn to_cookie(&self) -> String {
        format!("{}.{}.{}", self.state, self.nonce, self.verifier)
    }

    pub fn from_cookie(v: &str) -> Option<Self> {
        let mut it = v.split('.');
        let (state, nonce, verifier) = (it.next()?, it.next()?, it.next()?);
        let ok = |s: &str| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        };
        if it.next().is_some() || !ok(state) || !ok(nonce) || !ok(verifier) {
            return None;
        }
        Some(AuthRequest {
            state: state.into(),
            nonce: nonce.into(),
            verifier: verifier.into(),
        })
    }

    /// Constant-time check of the `state` echoed back by the IdP.
    pub fn state_matches(&self, returned: &str) -> bool {
        secret_eq(&self.state, returned)
    }
}

/// RFC 7636 S256: BASE64URL(SHA256(verifier)).
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn authorization_url(disc: &Discovery, cfg: &OidcConfig, req: &AuthRequest) -> String {
    let query: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code")
        .append_pair("client_id", &cfg.client_id)
        .append_pair("redirect_uri", &cfg.redirect_uri)
        .append_pair("scope", SCOPES)
        .append_pair("state", &req.state)
        .append_pair("nonce", &req.nonce)
        .append_pair("code_challenge", &pkce_challenge(&req.verifier))
        .append_pair("code_challenge_method", "S256")
        .finish();
    let sep = if disc.authorization_endpoint.contains('?') {
        '&'
    } else {
        '?'
    };
    format!("{}{sep}{query}", disc.authorization_endpoint)
}

#[derive(Debug, Deserialize)]
struct IdClaims {
    iss: String,
    aud: serde_json::Value,
    exp: f64,
    nonce: Option<String>,
    azp: Option<String>,
    sub: String,
}

/// Decodes the (unverified, TLS-delivered) id_token payload and checks
/// `iss`, `aud`, `exp` and `nonce`. Returns `sub`.
pub fn check_id_token(
    id_token: &str,
    issuer: &str,
    client_id: &str,
    nonce: &str,
    now_secs: u64,
) -> Result<String, String> {
    let mut parts = id_token.split('.');
    let payload = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(_), Some(p), Some(_), None) => p,
        _ => return Err("id_token is not a JWS compact token".into()),
    };
    let bytes = URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|_| "id_token payload is not base64url")?;
    let c: IdClaims =
        serde_json::from_slice(&bytes).map_err(|e| format!("id_token claims invalid: {e}"))?;
    if c.iss != issuer {
        return Err(format!("id_token issuer mismatch ({})", c.iss));
    }
    let auds: Vec<&str> = match &c.aud {
        serde_json::Value::String(s) => vec![s.as_str()],
        serde_json::Value::Array(a) => a.iter().filter_map(|v| v.as_str()).collect(),
        _ => vec![],
    };
    if !auds.contains(&client_id) {
        return Err("id_token audience does not include this client".into());
    }
    if auds.len() > 1 && c.azp.as_deref() != Some(client_id) {
        return Err("id_token azp does not match this client".into());
    }
    if c.exp <= now_secs as f64 {
        return Err("id_token has expired".into());
    }
    match c.nonce {
        Some(n) if secret_eq(&n, nonce) => {}
        _ => return Err("id_token nonce mismatch".into()),
    }
    if c.sub.is_empty() {
        return Err("id_token has no subject".into());
    }
    Ok(c.sub)
}

/// Why a sign-in failed; maps to an HTTP status on the error page.
#[derive(Debug)]
pub enum OidcError {
    /// Bad/missing state cookie, IdP returned an error, …
    BadRequest(String),
    /// Provider unreachable or misbehaving.
    Provider(String),
    /// Authenticated, but not allowed in.
    Denied(String),
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    id_token: String,
}

pub struct Oidc {
    pub config: OidcConfig,
    http: reqwest::Client,
    cache: RwLock<Option<(Instant, Discovery)>>,
}

/// HTTPS client with bundled Mozilla roots (the image is distroless).
fn http_client() -> reqwest::Client {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("rustls protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("yomu-server/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("reqwest client")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Oidc {
    pub fn new(config: OidcConfig) -> Self {
        Oidc {
            config,
            http: http_client(),
            cache: RwLock::new(None),
        }
    }

    /// The discovery document; fetched on demand (each sign-in attempt until
    /// one succeeds) and cached only after a successful, consistent fetch.
    pub async fn discovery(&self) -> Result<Discovery, String> {
        if let Some((at, d)) = &*self.cache.read().await {
            if at.elapsed() < DISCOVERY_TTL {
                return Ok(d.clone());
            }
        }
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.config.issuer.trim_end_matches('/')
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("discovery request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("discovery returned HTTP {}", resp.status()));
        }
        let d: Discovery = resp
            .json()
            .await
            .map_err(|e| format!("discovery document invalid: {e}"))?;
        if d.issuer.trim_end_matches('/') != self.config.issuer.trim_end_matches('/') {
            return Err(format!(
                "discovery issuer {} does not match YOMU_OIDC_ISSUER",
                d.issuer
            ));
        }
        *self.cache.write().await = Some((Instant::now(), d.clone()));
        Ok(d)
    }

    /// Completes the callback. Returns the admin identity string.
    pub async fn complete(
        &self,
        req: &AuthRequest,
        code: &str,
        returned_state: &str,
    ) -> Result<String, OidcError> {
        if !req.state_matches(returned_state) {
            return Err(OidcError::BadRequest("state mismatch".into()));
        }
        let disc = self.discovery().await.map_err(OidcError::Provider)?;
        let cfg = &self.config;

        // Code exchange.
        let basic = disc.use_basic_auth();
        let body = {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            form.append_pair("grant_type", "authorization_code")
                .append_pair("code", code)
                .append_pair("redirect_uri", &cfg.redirect_uri)
                .append_pair("code_verifier", &req.verifier);
            if !basic {
                form.append_pair("client_id", &cfg.client_id)
                    .append_pair("client_secret", &cfg.client_secret);
            }
            form.finish()
        };
        let mut request = self.http.post(&disc.token_endpoint);
        if basic {
            // RFC 6749 2.3.1: form-encode id and secret before Basic.
            let enc =
                |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
            request = request.basic_auth(enc(&cfg.client_id), Some(enc(&cfg.client_secret)));
        }
        let resp = request
            .header("content-type", "application/x-www-form-urlencoded")
            .header("accept", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| OidcError::Provider(format!("token request failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OidcError::Provider(format!(
                "token endpoint returned HTTP {status}: {}",
                body.chars().take(300).collect::<String>()
            )));
        }
        let tokens: TokenResponse = resp
            .json()
            .await
            .map_err(|e| OidcError::Provider(format!("token response invalid: {e}")))?;

        let sub = check_id_token(
            &tokens.id_token,
            &disc.issuer,
            &cfg.client_id,
            &req.nonce,
            now_secs(),
        )
        .map_err(OidcError::Provider)?;

        // Userinfo.
        let resp = self
            .http
            .get(&disc.userinfo_endpoint)
            .bearer_auth(&tokens.access_token)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|e| OidcError::Provider(format!("userinfo request failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(OidcError::Provider(format!(
                "userinfo returned HTTP {}",
                resp.status()
            )));
        }
        let info: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| OidcError::Provider(format!("userinfo invalid: {e}")))?;
        identity_from_userinfo(&info, &sub, cfg.admin_group.as_deref())
    }
}

/// Validates userinfo against the id_token subject and the optional admin
/// group, and builds the identity stored in the session.
pub fn identity_from_userinfo(
    info: &serde_json::Value,
    id_sub: &str,
    admin_group: Option<&str>,
) -> Result<String, OidcError> {
    let sub = info.get("sub").and_then(|v| v.as_str()).unwrap_or("");
    if sub != id_sub {
        return Err(OidcError::Provider(
            "userinfo subject does not match the id_token".into(),
        ));
    }
    let name = ["preferred_username", "email"]
        .iter()
        .find_map(|k| info.get(*k).and_then(|v| v.as_str()))
        .unwrap_or("unknown");
    if let Some(group) = admin_group {
        let member = info
            .get("groups")
            .and_then(|g| g.as_array())
            .is_some_and(|gs| gs.iter().any(|g| g.as_str() == Some(group)));
        if !member {
            return Err(OidcError::Denied(format!(
                "{name} is not in the admin group"
            )));
        }
    }
    Ok(format!("oidc:{sub} ({name})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn jwt(claims: serde_json::Value) -> String {
        let h = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let p = URL_SAFE_NO_PAD.encode(claims.to_string());
        format!("{h}.{p}.c2lnbmF0dXJl")
    }

    const ISS: &str = "https://idp.example/application/o/yomu/";

    fn good() -> serde_json::Value {
        json!({"iss": ISS, "aud": "yomu", "exp": 2_000_000_000u64, "nonce": "n1", "sub": "abc"})
    }

    fn check(c: serde_json::Value) -> Result<String, String> {
        check_id_token(&jwt(c), ISS, "yomu", "n1", 1_900_000_000)
    }

    #[test]
    fn pkce_rfc7636_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn auth_request_cookie_roundtrip() {
        let a = AuthRequest::generate();
        let b = AuthRequest::generate();
        assert_ne!(a, b);
        assert_eq!(a.verifier.len(), 43); // RFC 7636: 43..=128 chars
        assert_eq!(AuthRequest::from_cookie(&a.to_cookie()), Some(a.clone()));
        assert!(a.state_matches(&a.state));
        assert!(!a.state_matches(&b.state));
        assert!(!a.state_matches(""));
        for bad in ["", "a.b", "a.b.c.d", "a..c", "a.b.c d", "a.b.<c>"] {
            assert_eq!(AuthRequest::from_cookie(bad), None, "{bad}");
        }
    }

    #[test]
    fn authorization_url_has_pkce_and_params() {
        let disc = Discovery {
            issuer: ISS.into(),
            authorization_endpoint: "https://idp.example/authorize/".into(),
            token_endpoint: String::new(),
            userinfo_endpoint: String::new(),
            token_endpoint_auth_methods_supported: None,
        };
        let cfg = OidcConfig {
            issuer: ISS.into(),
            client_id: "yomu".into(),
            client_secret: "s".into(),
            redirect_uri: "https://yomu.example/admin/oidc/callback".into(),
            admin_group: None,
        };
        let req = AuthRequest::generate();
        let u = url::Url::parse(&authorization_url(&disc, &cfg, &req)).unwrap();
        let q: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["scope"], "openid profile email");
        assert_eq!(q["redirect_uri"], cfg.redirect_uri);
        assert_eq!(q["state"], req.state);
        assert_eq!(q["nonce"], req.nonce);
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["code_challenge"], pkce_challenge(&req.verifier));
        assert!(!u.as_str().contains(&req.verifier));
    }

    #[test]
    fn id_token_accepts_valid() {
        assert_eq!(check(good()).unwrap(), "abc");
        let mut c = good();
        c["aud"] = json!(["other", "yomu"]);
        c["azp"] = json!("yomu");
        assert_eq!(check(c).unwrap(), "abc");
    }

    #[test]
    fn id_token_rejects_bad_claims() {
        let cases: Vec<(&str, serde_json::Value)> = vec![
            ("iss", json!("https://evil.example/")),
            ("aud", json!("someone-else")),
            ("aud", json!(["a", "b"])),
            ("exp", json!(1_800_000_000u64)),
            ("nonce", json!("n2")),
        ];
        for (k, v) in cases {
            let mut c = good();
            c[k] = v;
            assert!(check(c).is_err(), "{k} should be rejected");
        }
        let mut c = good();
        c.as_object_mut().unwrap().remove("nonce");
        assert!(check(c).is_err());
        let mut c = good();
        c["aud"] = json!(["other", "yomu"]); // multiple audiences without azp
        assert!(check(c).is_err());
        assert!(check_id_token("not-a-jwt", ISS, "yomu", "n1", 0).is_err());
        assert!(check_id_token("a.!!!.c", ISS, "yomu", "n1", 0).is_err());
    }

    #[test]
    fn userinfo_identity_and_groups() {
        let info =
            json!({"sub": "abc", "preferred_username": "kanarek", "groups": ["yomu-admins"]});
        assert_eq!(
            identity_from_userinfo(&info, "abc", None).unwrap(),
            "oidc:abc (kanarek)"
        );
        assert!(identity_from_userinfo(&info, "abc", Some("yomu-admins")).is_ok());
        assert!(matches!(
            identity_from_userinfo(&info, "abc", Some("other")),
            Err(OidcError::Denied(_))
        ));
        assert!(identity_from_userinfo(&info, "xyz", None).is_err());
        let no_groups = json!({"sub": "abc", "email": "a@b"});
        assert_eq!(
            identity_from_userinfo(&no_groups, "abc", None).unwrap(),
            "oidc:abc (a@b)"
        );
        assert!(identity_from_userinfo(&no_groups, "abc", Some("g")).is_err());
    }

    #[test]
    fn auth_method_choice() {
        let mut d = Discovery {
            issuer: String::new(),
            authorization_endpoint: String::new(),
            token_endpoint: String::new(),
            userinfo_endpoint: String::new(),
            token_endpoint_auth_methods_supported: None,
        };
        assert!(d.use_basic_auth());
        d.token_endpoint_auth_methods_supported = Some(vec!["client_secret_post".into()]);
        assert!(!d.use_basic_auth());
        d.token_endpoint_auth_methods_supported = Some(vec![
            "client_secret_post".into(),
            "client_secret_basic".into(),
        ]);
        assert!(d.use_basic_auth());
    }
}
