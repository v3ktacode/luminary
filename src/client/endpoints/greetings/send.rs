// src/client/endpoints/greetings/send.rs
//
// Implementation of `SendGreetings`.
//
// Sends a single greeting of a given type to another player's profile and
// returns the server's `SendGreetingResult`.  If the server signals a
// logical failure (daily cap hit, unrecognised greeting type, …) even
// inside a 200 OK body, this function promotes it to a proper `MspError`
// so callers never have to inspect `result.success` themselves.
//

use serde_json::Value;

use crate::{
    client::http::{build_headers, ContentType},
    errors::{MspError, Result},
    models::SendGreetingResult,
};

use super::{EP, GreetingsEndpoint};

/// Sends one greeting to `profile_id` and returns the structured result.
///
/// Called exclusively by [`GreetingsEndpoint::send_greeting`]; lives here
/// to keep the public interface in `mod.rs` thin and readable.
pub(super) async fn send_greeting(
    ep:            &GreetingsEndpoint<'_>,
    greeting_type: &str,
    profile_id:    &str,
) -> Result<SendGreetingResult> {
    let session = ep.session.get().await?;

    // Resolve the correct cluster for this account's region.  EU accounts
    // hit `eu.mspapis.com/federationgateway/graphql`; US/CA accounts hit
    // `us.mspapis.com/federationgateway/graphql`.
    let url = ep.config.greetings_endpoint_regional(&session.region);

    // Persisted-query id for the send operation.  `ignoreDailyCap` is
    // always `false`; the server still enforces the cap regardless, but
    // some internal tooling paths pass `true` — we never do.
    let payload = serde_json::json!({
        "id": "SendGreetings-159BDD7706D824BB8F14874A7FAE3368",
        "variables": {
            "greetingType":      greeting_type,
            "receiverProfileId": profile_id,
            "ignoreDailyCap":    false,
        }
    });

    let response = ep
        .http
        .post(&url)
        .headers(build_headers(
            ContentType::Json,
            Some(&session.bearer()),
            "https://moviestarplanet2.com",
            "https://moviestarplanet2.com/",
        ))
        .json(&payload)
        .send()
        .await
        .map_err(|e| MspError::from_wreq(e, EP))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(MspError::api(EP, status.as_u16(), body));
    }

    // Deserialise as a raw `Value` first so we can navigate to the nested
    // object without needing a full mirror of the GraphQL envelope type.
    let bytes = response
        .bytes()
        .await
        .map_err(|e| MspError::from_wreq(e, EP))?;

    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|e| MspError::deserialize(e, EP))?;

    let raw = &body["data"]["greetings"]["sendGreeting"];

    let result: SendGreetingResult = serde_json::from_value(raw.clone())
        .map_err(|e| MspError::deserialize(e, EP))?;

    // The server can return HTTP 200 with `success: false` when a logical
    // constraint is violated (daily cap, unknown type, …).  We surface
    // this as a proper error so callers never have to inspect the flag.
    if !result.success {
        if let Some(ref err) = result.error {
            return Err(MspError::api(
                EP,
                200,
                format!(
                    "reason={}, next_in={}s, message={}",
                    err.reason,
                    err.next_greeting_seconds_remaining
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "N/A".to_owned()),
                    err.message,
                ),
            ));
        }
    }

    Ok(result)
}