// src/client/endpoints/greetings/definitions.rs
//
// Implementation of `GetGreetingsDefinitions`.
//
// Sends a persisted-query request to the federation gateway and deserialises
// the response into a `Vec<GreetingDefinition>`.
//

use serde_json::Value;

use crate::{
    client::http::{build_headers, ContentType},
    errors::{MspError, Result},
    models::GreetingDefinition,
};

use super::{EP, GreetingsEndpoint};

/// Fetches the full greeting-type catalogue for the authenticated player.
///
/// Called exclusively by [`GreetingsEndpoint::get_greeting_definitions`];
/// lives here to keep the public interface in `mod.rs` thin and readable.
pub(super) async fn get_greeting_definitions(
    ep: &GreetingsEndpoint<'_>,
) -> Result<Vec<GreetingDefinition>> {
    let session = ep.session.get().await?;

    // Resolve the correct cluster for this account's region.  EU accounts
    // hit `eu.mspapis.com/federationgateway/graphql`; US/CA accounts hit
    // `us.mspapis.com/federationgateway/graphql`.
    let url = ep.config.greetings_endpoint_regional(&session.region);

    // Persisted-query id issued by the MSP2 web client for this operation.
    // The `variables` field is intentionally empty — the server derives the
    // caller's identity from the bearer token alone.
    let payload = serde_json::json!({
        "id":        "GetGreetingsDefinitions-5FBA528E623526E9F8378521DC7F0623",
        "variables": ""
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
    // array without needing a full mirror of the GraphQL envelope type.
    let bytes = response
        .bytes()
        .await
        .map_err(|e| MspError::from_wreq(e, EP))?;

    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|e| MspError::deserialize(e, EP))?;

    let raw = &body["data"]["profiles"]["me"]["greetings"]["definitions"];

    let definitions: Vec<GreetingDefinition> = serde_json::from_value(raw.clone())
        .map_err(|e| MspError::deserialize(e, EP))?;

    Ok(definitions)
}