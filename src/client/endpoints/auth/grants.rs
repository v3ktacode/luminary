// src/client/endpoints/auth/grants.rs
//
// The three OAuth operations that make up the login flow:
//
//   1. password_grant     — trades username + password for a generic token pair.
//   2. fetch_profile_id   — resolves the JWT `sub` to an in-game `profileId`.
//   3. refresh_grant      — trades a refresh token for a profile-scoped session.
//
// `run_login_flow` stitches those three together and wraps the whole thing
// in the retry loop.
//
// These functions are free functions rather than methods because they need
// to be called from both `mod.rs` (initial login) and `relogin.rs`
// (automatic re-login), and threading `&AuthEndpoint` through a background
// task would require owning all the fields anyway.

use serde::Serialize;
use serde_json::Value;

use super::{
    credentials, timeouts, AuthEndpoint, EP, GAME_ID, ORIGIN,
};
use crate::{
    client::http::{build_headers, ContentType},
    errors::{MspError, Result},
    models::MspSession,
};

// When the JWT doesn't contain an `exp` claim (shouldn't happen in practice,
// but the server has surprised us before), we assume the token lives for one
// hour.
const DEFAULT_TOKEN_LIFETIME_SECS: i64 = 3600;

// ---------------------------------------------------------------------------
// JWT helpers
// ---------------------------------------------------------------------------

/// Utilities for extracting claims from a JWT without verifying the signature.
///
/// We trust the token endpoint to give us a valid token; we only need the
/// claims to know *who* the token belongs to (`sub`) and *when* it expires
/// (`exp`).
mod jwt {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use serde_json::Value;

    use crate::errors::{MspError, Result};

    /// Returns the `sub` (subject) claim — the unique identifier for the
    /// login identity, used to look up the associated game profiles.
    pub fn decode_sub(token: &str) -> Result<String> {
        claims(token)?
            .get("sub")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| MspError::Jwt("JWT missing 'sub' claim".into()))
    }

    /// Returns the `exp` (expiry) claim as a Unix timestamp in seconds.
    pub fn decode_exp(token: &str) -> Result<i64> {
        claims(token)?
            .get("exp")
            .and_then(Value::as_i64)
            .ok_or_else(|| MspError::Jwt("JWT missing 'exp' claim".into()))
    }

    /// Decodes the payload segment of a JWT and returns it as a JSON value.
    /// Signature verification is intentionally skipped.
    fn claims(token: &str) -> Result<Value> {
        let payload = token
            .split('.')
            .nth(1)
            .ok_or_else(|| MspError::Jwt("Malformed JWT — missing payload segment".into()))?;

        let bytes = URL_SAFE_NO_PAD.decode(payload)?;
        serde_json::from_slice(&bytes).map_err(|e| MspError::deserialize(e, super::EP))
    }
}

// ---------------------------------------------------------------------------
// OAuth error types
// ---------------------------------------------------------------------------

/// The error body returned by the `/connect/token` endpoint on failure.
///
/// Two shapes are seen in the wild:
///
/// ```json
/// { "error": "invalid_grant",
///   "error_description": "invalid_username_or_password" }
///
/// { "error": "login_denied",
///   "error_description": "PermanentlyBanned",
///   "reasonPhrase": "loginDeactivated" }
/// ```
///
/// `reason_phrase` defaults to an empty string when absent so we can match
/// on it uniformly.
#[derive(Debug, serde::Deserialize)]
struct OAuthError {
    error: String,
    #[serde(default)]
    error_description: String,
    /// Only present on `login_denied` responses (e.g. `"loginDeactivated"`).
    #[serde(default, rename = "reasonPhrase")]
    reason_phrase: String,
}

impl OAuthError {
    /// Converts this wire-format error into the most specific `MspError`
    /// variant we have.
    ///
    /// `username` and `region` end up in the error message so callers can
    /// tell which account the failure belongs to.  For the password grant,
    /// pass the raw login name; for refresh grants, pass the `profile_id`.
    fn into_msp_error(self, status: u16, username: &str, region: &str) -> MspError {
        match self.error.as_str() {
            "invalid_grant"
                if self.error_description == "invalid_username_or_password" =>
            {
                MspError::InvalidCredentials {
                    username: username.to_owned(),
                    region: region.to_owned(),
                }
            }

            "login_denied" if self.reason_phrase == "loginDeactivated" => {
                MspError::AccountBanned {
                    username: username.to_owned(),
                    region: region.to_owned(),
                    reason: self.error_description,
                }
            }

            "login_denied" => MspError::AuthFailed {
                username: username.to_owned(),
                region: region.to_owned(),
                reason: format!("{} ({})", self.reason_phrase, self.error_description),
            },

            _ => MspError::api(
                EP,
                status,
                format!("OAuth error '{}': {}", self.error, self.error_description),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

fn wreq_err(e: wreq::Error) -> MspError {
    MspError::from_wreq(e, EP)
}


/// Like `read_json`, but understands the OAuth error shapes returned by the
/// `/connect/token` endpoint.
///
/// On non-2xx responses we first try to deserialise the body as an
/// `OAuthError` and convert it to the right `MspError` variant.  If that
/// fails we fall back to a plain `MspError::Api`.
async fn read_token_response(
    response: wreq::Response,
    username: &str,
    region: &str,
) -> Result<Value> {
    let status = response.status();

    if !status.is_success() {
        let status_u16 = status.as_u16();
        // Buffer the body once so we can try two different parsers.
        let bytes = response.bytes().await.unwrap_or_default();

        if let Ok(oauth_err) = serde_json::from_slice::<OAuthError>(&bytes) {
            return Err(oauth_err.into_msp_error(status_u16, username, region));
        }

        let body = String::from_utf8_lossy(&bytes).into_owned();
        return Err(MspError::api(EP, status_u16, body));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(wreq_err)?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|e| MspError::deserialize(e, EP))?;
    crate::client::http::ensure_no_api_error(value, EP)
}

/// Pulls a required string field out of a JSON object.
///
/// Returns `MspError::Api(422)` if the field is absent or not a string.
fn required_str(value: &Value, field: &str) -> Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| MspError::api(EP, 422, format!("Missing field '{field}'")))
}

/// Current Unix time in seconds.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// ---------------------------------------------------------------------------
// Backoff helper (shared with relogin.rs via super)
// ---------------------------------------------------------------------------

/// Returns a random duration between zero and `max_ms` milliseconds.
pub(super) fn jitter(max_ms: u64) -> std::time::Duration {
    use rand::Rng;
    std::time::Duration::from_millis(rand::thread_rng().gen_range(0..max_ms))
}

/// Exponential back-off capped at `max`, with up to 500 ms of random jitter.
pub(super) fn backoff_with_jitter(
    base: std::time::Duration,
    max: std::time::Duration,
    attempt: u32,
) -> std::time::Duration {
    let exp = base.saturating_mul(1u32 << attempt.saturating_sub(1).min(16));
    exp.min(max) + jitter(500)
}

// ---------------------------------------------------------------------------
// acr_values helper
// ---------------------------------------------------------------------------

/// Builds the base `acr_values` string shared by all grant requests.
///
/// The server uses this to associate the resulting token with a specific
/// game and device.  The `profileId` segment is appended on top of this
/// base when making a refresh grant.
pub(super) fn acr_base(endpoint: &AuthEndpoint<'_>) -> String {
    format!("gameId:{GAME_ID} deviceId:{}", endpoint.device_id)
}

// ---------------------------------------------------------------------------
// Login flow
// ---------------------------------------------------------------------------

/// Runs the full login flow (password grant → profile lookup → refresh grant)
/// with the configured retry policy.
///
/// Currently `LOGIN_MAX_ATTEMPTS` is 1 (no retries) — the retry
/// infrastructure is already in place if we ever want to raise that constant.
pub(super) async fn run_login_flow(
    endpoint: &AuthEndpoint<'_>,
    username: &str,
    password: &str,
    region: &str,
) -> Result<MspSession> {
    use timeouts::{LOGIN_BACKOFF_BASE, LOGIN_BACKOFF_MAX, LOGIN_MAX_ATTEMPTS};

    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match login_flow_once(endpoint, username, password, region).await {
            Ok(session) => return Ok(session),
            Err(e) if attempt >= LOGIN_MAX_ATTEMPTS => {
                tracing::error!(attempt, "Login flow exhausted retries.");
                return Err(e);
            }
            Err(e) => {
                let delay = backoff_with_jitter(LOGIN_BACKOFF_BASE, LOGIN_BACKOFF_MAX, attempt);
                tracing::warn!(
                    attempt,
                    retry_in_ms = delay.as_millis(),
                    "Login failed: {e:?}. Retrying."
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// Executes a single, complete login attempt:
/// password grant → profile lookup → profile-scoped refresh grant.
async fn login_flow_once(
    endpoint: &AuthEndpoint<'_>,
    username: &str,
    password: &str,
    region: &str,
) -> Result<MspSession> {
    let base = acr_base(endpoint);

    tracing::debug!("Running password grant…");
    let (access_token, refresh_token, sub) =
        password_grant(endpoint, region, username, password, &base).await?;

    tracing::debug!(sub = %sub, "Resolving profile ID…");
    let profile_id = fetch_profile_id(endpoint, &sub, region, &access_token).await?;

    tracing::debug!(
        profile_id = %profile_id,
        "Exchanging refresh token for profile-scoped session…"
    );
    refresh_grant(endpoint, &refresh_token, &base, &profile_id, &sub, region).await
}

// ---------------------------------------------------------------------------
// Grant methods
// ---------------------------------------------------------------------------

/// Performs the OAuth2 password grant and returns
/// `(access_token, refresh_token, sub)`.
///
/// The `sub` is the unique identifier for the *login identity* (not the game
/// profile) — we use it in the next step to find the `profileId`.
async fn password_grant(
    endpoint: &AuthEndpoint<'_>,
    region: &str,
    username: &str,
    password: &str,
    acr_base: &str,
) -> Result<(String, String, String)> {
    #[derive(Serialize)]
    struct Body<'a> {
        client_id:     &'a str,
        client_secret: &'a str,
        grant_type:    &'a str,
        scope:         &'a str,
        username:      String,
        password:      &'a str,
        acr_values:    &'a str,
    }

    // The server identifies the region via the username prefix, not a
    // separate field, so we format it as `"REGION|username"`.
    let body = serde_urlencoded::to_string(Body {
        client_id:     credentials::client_id(),
        client_secret: credentials::client_secret(),
        grant_type:    "password",
        scope:         "openid nebula offline_access",
        username:      format!("{region}|{username}"),
        password,
        acr_values:    acr_base,
    })?;

    let value = post_token_form(endpoint, region, body, None, username, region).await?;

    let access_token  = required_str(&value, "access_token")?;
    let refresh_token = required_str(&value, "refresh_token")?;
    let sub           = jwt::decode_sub(&access_token)?;

    Ok((access_token, refresh_token, sub))
}

/// Resolves a login `sub` to an in-game `profileId`.
///
/// One login identity can have multiple game profiles (though in practice
/// MSP2 accounts have exactly one).  We always take the first result.
pub(super) async fn fetch_profile_id(
    endpoint: &AuthEndpoint<'_>,
    sub: &str,
    region: &str,
    bearer_token: &str,
) -> Result<String> {
    let url = format!(
        "{}?region={region}&pageSize=1",
        endpoint.config.profiles_for_sub_regional(sub, region)
    );
    let bearer  = format!("Bearer {bearer_token}");
    let headers = build_headers(
        ContentType::Json,
        Some(&bearer),
        ORIGIN,
        &endpoint.config.referer,
    );

    let response: Value = endpoint
        .http
        .get(&url)
        .headers(headers)
        .send()
        .await
        .map_err(wreq_err)?
        .json()
        .await
        .map_err(wreq_err)?;

    response
        .get(0)
        .and_then(|p| p.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            MspError::api(
                EP,
                404,
                format!("Profile resolution failed — sub='{sub}' region='{region}'"),
            )
        })
}

/// Exchanges a refresh token for a profile-scoped session.
///
/// The `profileId` is embedded in `acr_values` so the server issues an
/// access token scoped to that specific game character.  This is the last
/// step of both the initial login flow and every automatic re-login.
pub(super) async fn refresh_grant(
    endpoint: &AuthEndpoint<'_>,
    refresh_token: &str,
    acr_base: &str,
    profile_id: &str,
    sub_id: &str,
    region: &str,
) -> Result<MspSession> {
    #[derive(Serialize)]
    struct Body<'a> {
        grant_type:    &'a str,
        refresh_token: &'a str,
        acr_values:    String,
    }

    let body = serde_urlencoded::to_string(Body {
        grant_type:    "refresh_token",
        refresh_token,
        acr_values:    format!("{acr_base} profileId:{profile_id}"),
    })?;

    let auth  = credentials::basic_auth();
    let value = post_token_form(
        endpoint,
        region,
        body,
        Some(&auth),
        profile_id,
        region,
    )
    .await?;

    let access_token  = required_str(&value, "access_token")?;
    let refresh_token = required_str(&value, "refresh_token")?;

    // Prefer the expiry embedded in the JWT itself; fall back to a safe
    // default if the claim is missing.
    let expires_at = jwt::decode_exp(&access_token)
        .unwrap_or_else(|_| unix_now() + DEFAULT_TOKEN_LIFETIME_SECS);

    Ok(MspSession {
        access_token,
        refresh_token,
        profile_id:              profile_id.to_owned(),
        sub_id:                  sub_id.to_owned(),
        device_id:               endpoint.device_id.to_owned(),
        access_token_expires_at: expires_at,
        region:                  region.to_owned(),
    })
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

/// POSTs a `application/x-www-form-urlencoded` body to the token endpoint
/// and routes the response through OAuth-aware error parsing.
///
/// `username` and `region` are only used to populate error messages — pass
/// the raw login name for password grants and `profile_id` for refresh grants.
async fn post_token_form(
    endpoint: &AuthEndpoint<'_>,
    region: &str,
    body: String,
    auth: Option<&str>,
    username: &str,
    error_region: &str,
) -> Result<Value> {
    let headers = build_headers(
        ContentType::Form,
        auth,
        ORIGIN,
        &endpoint.config.referer,
    );
    let response = endpoint
        .http
        .post(endpoint.config.token_endpoint_regional(region))
        .headers(headers)
        .body(body)
        .send()
        .await
        .map_err(wreq_err)?;

    read_token_response(response, username, error_region).await
}