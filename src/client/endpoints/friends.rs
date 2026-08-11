// src/client/endpoints/friends.rs
//
// Friends and friend requests, split across two different backend styles:
//
//   • Reading relationship state (friend list, pending requests, blocked, …)
//     goes through a GraphQL gateway (`edgerelationships/graphql`).
//   • Responding to a specific friend request (accept/reject) goes through
//     a plain REST endpoint (`profilerelationships/v2/...`).
//
// Both are region-aware, routed via the logged-in session's region.
//
// IMPORTANT — do not split or rewrite the GraphQL query below.
// The server may fingerprint the exact query string. We always send the
// full `GetAllRelationships` query as observed from the official Unity
// client, even when a given method only reads one field out of the
// response. Sending a trimmed-down query risks being flagged as a
// non-official client.

use std::sync::Arc;

use serde_json::{json, Value};
use wreq::Client;

use super::super::http::{build_headers, decode_response_value, ContentType};
use crate::{
    config::MspConfig,
    errors::{MspError, Result},
    models::MspSession,
    session::SessionStore,
};

/// Short label attached to every `MspError` raised from this module.
const EP: &str = "friends";

const ORIGIN: &str = "https://moviestarplanet2.com";
const REFERER: &str = "https://moviestarplanet2.com/";

/// The exact query string sent by the official Unity client.
///
/// Do not trim or reformat this. Even though some methods only read one
/// field out of the response (e.g. `get_friends` only reads
/// `relationships.nodes`, `get_pending_requests` only reads
/// `requestsIn.nodes`), we always request the full payload. Sending a
/// reduced query could cause the server to flag the request as coming
/// from an unofficial client.
const GET_ALL_RELATIONSHIPS_QUERY: &str = "\
query GetAllRelationships($profileId: String!, $gameId: String!){\
 relationships(profileId: $profileId) { nodes { profileId labels(gameId: $gameId) } } \
 requestsIn(profileId: $profileId) { nodes { profileId } } \
 requestsOut(profileId: $profileId) { nodes { profileId } } \
 blocked(profileId: $profileId) { nodes { profileId } } \
 labelRequestsIn(profileId: $profileId, gameId: $gameId) { nodes { profileId label } } \
 labelRequestsOut(profileId: $profileId, gameId: $gameId) { nodes { profileId label } }\
}";

pub struct FriendsEndpoint<'c> {
    pub(crate) http: &'c Client,
    pub(crate) session: &'c SessionStore,
    pub(crate) config: Arc<MspConfig>,
}

impl<'c> FriendsEndpoint<'c> {
    /// Lists the friend ids for a profile.
    ///
    /// Pass `None` to look up the logged-in user's own friends, or
    /// `Some(id)` for any other profile.
    ///
    /// Sends the full `GetAllRelationships` query and reads only the
    /// `relationships.nodes` field from the response — see the module-level
    /// note for why we don't send a trimmed query.
    #[tracing::instrument(name = "friends.get", skip(self))]
    pub async fn get_friends(&self, profile_id: Option<&str>) -> Result<Vec<String>> {
        let session = self.session.get().await?;
        let target_id = profile_id.unwrap_or(&session.profile_id).to_owned();

        let data = self.get_all_relationships(&session, &target_id).await?;

        let nodes = data["data"]["relationships"]["nodes"]
            .as_array()
            .ok_or_else(|| {
                MspError::api(EP, 422, "Missing 'data.relationships.nodes' in GraphQL response")
            })?;

        Ok(extract_profile_ids(nodes))
    }

    /// Sends the full `GetAllRelationships` query and reads only the
    /// `requestsIn.nodes` field from the response — see the module-level
    /// note for why we don't send a trimmed query.
    #[tracing::instrument(name = "friends.get_pending_requests", skip(self))]
    pub async fn get_pending_requests(&self) -> Result<Vec<String>> {
        let session = self.session.get().await?;
        let profile_id = session.profile_id.clone();

        let data = self.get_all_relationships(&session, &profile_id).await?;

        let nodes = data["data"]["requestsIn"]["nodes"].as_array().ok_or_else(|| {
            MspError::api(EP, 422, "Missing 'data.requestsIn.nodes' in GraphQL response")
        })?;

        Ok(extract_profile_ids(nodes))
    }

    /// Accepts a pending friend request from `requester_profile_id`.
    #[tracing::instrument(name = "friends.accept_request", skip(self))]
    pub async fn accept_request(&self, requester_profile_id: &str) -> Result<()> {
        self.set_request_state(requester_profile_id, "approved").await
    }

    /// Rejects a pending friend request from `requester_profile_id`.
    #[tracing::instrument(name = "friends.reject_request", skip(self))]
    pub async fn reject_request(&self, requester_profile_id: &str) -> Result<()> {
        self.set_request_state(requester_profile_id, "rejected").await
    }

    // -------------------------------------------------------------------
    // Internal helpers
    // -------------------------------------------------------------------

    /// Sends the full `GetAllRelationships` query for `profile_id` and
    /// returns the raw decoded JSON.
    ///
    /// All public methods in this endpoint go through this single function
    /// so that the exact query string is never accidentally changed when
    /// adding a new method.
    async fn get_all_relationships(&self, session: &MspSession, profile_id: &str) -> Result<Value> {
        let url = self.config.relationships_graphql_regional(&session.region);
        let payload = json!({
            "query": GET_ALL_RELATIONSHIPS_QUERY,
            "variables": {
                "profileId": profile_id,
                "gameId":    self.config.game_id,
            }
        });

        let response = self
            .http
            .post(&url)
            .headers(build_headers(
                ContentType::Json,
                Some(&session.bearer()),
                ORIGIN,
                REFERER,
            ))
            .json(&payload)
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        decode_response_value(response, EP).await
    }

    /// Accepts or rejects a friend request by PUTting the new state onto
    /// the relationship's REST resource.
    ///
    /// `requester_profile_id` is whoever sent the original request; the
    /// logged-in user is always the one responding to it.
    async fn set_request_state(&self, requester_profile_id: &str, state: &str) -> Result<()> {
        let session = self.session.get().await?;
        let url = self.config.relationship_request_regional(
            requester_profile_id,
            &session.profile_id,
            &session.region,
        );

        let payload = json!({
            "profileId": session.profile_id,
            "state":     state,
        });

        let response = self
            .http
            .put(&url)
            .headers(build_headers(
                ContentType::Json,
                Some(&session.bearer()),
                ORIGIN,
                REFERER,
            ))
            .json(&payload)
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        ensure_success(response, EP).await
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Pulls the `profileId` string out of each GraphQL node, silently skipping
/// any node that's missing it. In practice this should never drop anything —
/// it's just cheap insurance against a slightly off-shape server response.
fn extract_profile_ids(nodes: &[Value]) -> Vec<String> {
    nodes
        .iter()
        .filter_map(|node| {
            node.get("profileId")
                .and_then(Value::as_str)
                .map(String::from)
        })
        .collect()
}

/// Checks a response's status code without attempting to decode a body.
///
/// Used for accept/reject, which don't return anything meaningful on
/// success — only the status code matters.
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