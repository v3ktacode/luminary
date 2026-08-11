// src/client/endpoints/collects.rs
//
// "Collects" and "time-limited rewards" are two related but distinct
// reward systems on the profile:
//
//   • Time-limited rewards (`claim_reward`) — things like the daily login
//     pickup. You just PUT a "Claimed" state against a named reward type
//     and the server marks it as collected. No response body to parse.
//
//   • Collects (`get_collects` / `claim_collects`) — a queue of pending
//     rewards (XP, currency, …) that accumulate from regular gameplay and
//     need to be explicitly claimed in bulk by collect type. `get_collects`
//     lists what's currently pending; `claim_collects` cashes a chosen
//     subset of them in.
//
// The two systems don't share an endpoint or a payload shape, which is why
// this file has two separate code paths rather than one generic "claim"
// method.

use std::sync::Arc;
use wreq::Client;

use super::super::http::{build_headers, decode_response, ContentType};
use crate::config::MspConfig;
use crate::errors::{MspError, Result};
use crate::models::Collect;
use crate::session::SessionStore;

/// Short label attached to every `MspError` raised from this module.
const EP: &str = "collects";

/// Reward type identifiers understood by the time-limited-rewards endpoint.
/// wraps in a named method — anything else would need to go through the
/// underlying API directly.
const REWARD_DAILY_PICKUP: &str = "daily_pickup";
const REWARD_DAILY_PICKUP_VIP: &str = "daily_pickup_vip";

pub struct CollectsEndpoint<'c> {
    pub(crate) http: &'c Client,
    pub(crate) session: &'c SessionStore,
    pub(crate) config: Arc<MspConfig>,
}

impl<'c> CollectsEndpoint<'c> {
    /// Claims the daily login pickup reward.
    #[tracing::instrument(name = "collects.pickup", skip(self))]
    pub async fn collect_pickup(&self) -> Result<()> {
        self.claim_reward(REWARD_DAILY_PICKUP).await
    }

    /// Claims the VIP variant of the daily login pickup reward.
    #[tracing::instrument(name = "collects.pickup_vip", skip(self))]
    pub async fn collect_pickup_vip(&self) -> Result<()> {
        self.claim_reward(REWARD_DAILY_PICKUP_VIP).await
    }

    /// Lists the profile's currently pending collects (unclaimed XP /
    /// currency rewards accumulated from gameplay).
    #[tracing::instrument(name = "collects.get", skip(self))]
    pub async fn get_collects(&self) -> Result<Vec<Collect>> {
        let session = self.session.get().await?;
        let url = self.config.collects(&session.profile_id);

        let response = self
            .http
            .get(&url)
            .headers(self.headers(&session.bearer()))
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        decode_response(response, EP).await
    }

    /// Claims a specific set of pending collects by their collect type,
    /// returning the ones that were successfully claimed.
    #[tracing::instrument(name = "collects.claim", skip(self))]
    pub async fn claim_collects(&self, collect_list: &[&str]) -> Result<Vec<Collect>> {
        let session = self.session.get().await?;
        let url = self.config.collects_claim(&session.profile_id);
        let payload = serde_json::json!({ "collectTypes": collect_list });

        let response = self
            .http
            .post(&url)
            .headers(self.headers(&session.bearer()))
            .json(&payload)
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        decode_response(response, EP).await
    }

    // -------------------------------------------------------------------
    // Internal helpers
    // -------------------------------------------------------------------

    /// Marks a time-limited reward as claimed.
    ///
    /// Unlike the collects endpoints above, this one has no response body
    /// worth parsing — a 2xx just means the claim went through — so we only
    /// check the status code rather than decoding JSON.
    async fn claim_reward(&self, reward_type: &str) -> Result<()> {
        let session = self.session.get().await?;
        let url = self.config.time_limited_reward(&session.profile_id, reward_type);
        let payload = serde_json::json!({ "state": "Claimed" });

        let response = self
            .http
            .put(&url)
            .headers(self.headers(&session.bearer()))
            .json(&payload)
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        ensure_success(response, EP).await
    }

    /// Builds the standard JSON + bearer header set used by every request
    /// in this endpoint.
    fn headers(&self, bearer: &str) -> wreq::header::HeaderMap {
        build_headers(
            ContentType::Json,
            Some(bearer),
            &self.config.origin,
            &self.config.referer,
        )
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Checks a response's status code without attempting to decode a body.
///
/// This mirrors the non-2xx handling in `http::decode_response` (including
/// `429` rate-limit detection) for endpoints that don't return a JSON body
/// worth parsing on success — `claim_reward` being the only one here.
async fn ensure_success(response: wreq::Response, endpoint: &'static str) -> Result<()> {
    let status = response.status();

    if status.as_u16() == 429 {
        let retry_after_secs = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60);
        return Err(MspError::RateLimited { endpoint, retry_after_secs });
    }

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(MspError::api(endpoint, status.as_u16(), body));
    }

    Ok(())
}