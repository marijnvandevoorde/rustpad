//! Verification of Cloudflare Access tokens, for the private document list.
//!
//! Cloudflare Access sits in front of the list and adds a signed JWT to each
//! request in the `Cf-Access-Jwt-Assertion` header. The server checks that
//! signature itself, so the list stays private even if someone reaches the
//! origin without going through Access. The unsigned
//! `Cf-Access-Authenticated-User-Email` header is never trusted.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use log::warn;
use serde::Deserialize;
use tokio::sync::RwLock;

/// The header Cloudflare Access adds to requests it let through.
pub const HEADER: &str = "cf-access-jwt-assertion";

/// Refetch the signing keys at most this often.
const KEYS_TTL: Duration = Duration::from_secs(3600);

/// Refetch early for an unknown key id at most this often.
const KEYS_MIN_REFRESH: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
struct Claims {
    email: Option<String>,
}

struct CachedKeys {
    fetched: Instant,
    keys: JwkSet,
}

/// Checks Cloudflare Access tokens for one Access application.
#[derive(Clone)]
pub struct AccessVerifier {
    inner: Arc<Inner>,
}

struct Inner {
    issuer: String,
    audience: String,
    allowed_emails: Vec<String>,
    certs_url: String,
    client: reqwest::Client,
    keys: RwLock<Option<CachedKeys>>,
}

impl std::fmt::Debug for AccessVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessVerifier")
            .field("issuer", &self.inner.issuer)
            .finish_non_exhaustive()
    }
}

impl AccessVerifier {
    /// A verifier for tokens of the Access team `team_domain` (for example
    /// `example.cloudflareaccess.com`) and the application with AUD tag
    /// `audience`. When `allowed_emails` is not empty, only those users pass.
    pub fn new(team_domain: &str, audience: &str, allowed_emails: &[String]) -> Self {
        let team_domain = team_domain.trim().trim_end_matches('/');
        Self::with_certs_url(
            team_domain,
            audience,
            allowed_emails,
            &format!("https://{team_domain}/cdn-cgi/access/certs"),
        )
    }

    /// Like [`Self::new`], with the URL of the signing keys given explicitly.
    pub fn with_certs_url(
        team_domain: &str,
        audience: &str,
        allowed_emails: &[String],
        certs_url: &str,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("failed to build HTTP client");
        Self {
            inner: Arc::new(Inner {
                issuer: format!("https://{}", team_domain.trim().trim_end_matches('/')),
                audience: audience.trim().to_owned(),
                allowed_emails: allowed_emails
                    .iter()
                    .map(|e| e.trim().to_lowercase())
                    .filter(|e| !e.is_empty())
                    .collect(),
                certs_url: certs_url.to_owned(),
                client,
                keys: RwLock::new(None),
            }),
        }
    }

    /// Verifies a token and returns the user's email address.
    pub async fn verify(&self, token: &str) -> Result<String> {
        let header = decode_header(token).context("malformed token")?;
        if header.alg != Algorithm::RS256 {
            bail!("unexpected token algorithm {:?}", header.alg);
        }
        let kid = header.kid.ok_or_else(|| anyhow!("token has no key id"))?;

        let mut keys = self.keys(false).await?;
        if keys.find(&kid).is_none() {
            // Keys rotate; look for a new one before giving up.
            keys = self.keys(true).await?;
        }
        let jwk = keys
            .find(&kid)
            .ok_or_else(|| anyhow!("token signed with unknown key {kid}"))?;
        let key = DecodingKey::from_jwk(jwk).context("unusable signing key")?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.inner.audience]);
        validation.set_issuer(&[&self.inner.issuer]);
        validation.set_required_spec_claims(&["exp", "aud", "iss"]);
        let claims = decode::<Claims>(token, &key, &validation)
            .context("invalid token")?
            .claims;

        let email = claims.email.unwrap_or_default().trim().to_lowercase();
        if email.is_empty() {
            bail!("token carries no email");
        }
        // A second gate behind the Access policy, in case it is widened.
        if !self.inner.allowed_emails.is_empty() && !self.inner.allowed_emails.contains(&email) {
            bail!("{email} is not allowed");
        }
        Ok(email)
    }

    /// Returns the signing keys, fetched at most once per [`KEYS_TTL`], or
    /// sooner when `refresh` is set (at most once per [`KEYS_MIN_REFRESH`]).
    async fn keys(&self, refresh: bool) -> Result<JwkSet> {
        let max_age = if refresh { KEYS_MIN_REFRESH } else { KEYS_TTL };
        if let Some(cached) = &*self.inner.keys.read().await {
            if cached.fetched.elapsed() < max_age {
                return Ok(cached.keys.clone());
            }
        }

        let mut cache = self.inner.keys.write().await;
        if let Some(cached) = &*cache {
            if cached.fetched.elapsed() < max_age {
                return Ok(cached.keys.clone());
            }
        }
        match self.fetch_keys().await {
            Ok(keys) => {
                *cache = Some(CachedKeys {
                    fetched: Instant::now(),
                    keys: keys.clone(),
                });
                Ok(keys)
            }
            // Use stale keys rather than lock everyone out while Cloudflare is
            // briefly unreachable; keys rotate over weeks.
            Err(e) => match &*cache {
                Some(cached) => {
                    warn!("using stale Access signing keys: {e:#}");
                    Ok(cached.keys.clone())
                }
                None => Err(e),
            },
        }
    }

    async fn fetch_keys(&self) -> Result<JwkSet> {
        self.inner
            .client
            .get(&self.inner.certs_url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .context("could not fetch Access signing keys")?
            .json::<JwkSet>()
            .await
            .context("unusable Access signing keys")
    }
}
