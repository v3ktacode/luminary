// src/client/endpoints/greetings/mod.rs
//
// Greetings endpoint — fetching greeting definitions and sending greetings
// to other players.
//
// The game exposes two GraphQL operations for this feature:
//
//   • `GetGreetingsDefinitions` — returns the catalogue of all greeting
//     types the authenticated player has access to (id, cost, …).
//   • `SendGreetings` — sends one greeting of a given type to another
//     player's profile.
//
// This module is split into two files:
//
//   mod.rs          — this file: the endpoint struct and its public API.
//   definitions.rs  — `get_greeting_definitions`: fetches the catalogue.
//   send.rs         — `send_greeting`: sends one greeting to a profile.

mod definitions;
mod send;

use std::sync::Arc;

use wreq::Client;

use crate::config::MspConfig;
use crate::session::SessionStore;

/// Short label attached to every `MspError` raised from this module.
pub(super) const EP: &str = "greetings";

/// Greetings endpoint handle.
///
/// Obtained via [`crate::client::MspClient::greetings`]; holds a shared
/// reference to the underlying HTTP client and the session store so it
/// can attach the correct `Authorization` header on every request.
///
/// The `config` field is used to resolve the correct regional federation-
/// gateway URL at runtime (EU by default; US/CA accounts hit the US
/// cluster).
pub struct GreetingsEndpoint<'c> {
    pub(crate) http:    &'c Client,
    pub(crate) session: &'c SessionStore,
    pub(crate) config:  Arc<MspConfig>,
}

impl<'c> GreetingsEndpoint<'c> {
    /// Returns the full catalogue of greeting definitions available to the
    /// authenticated player.
    ///
    /// Each `GreetingDefinition` describes one greeting type: its internal
    /// id, the cost, and so on.
    ///
    /// # Errors
    ///
    /// Returns `MspError::Api` when the server responds with a non-2xx
    /// status, or `MspError::Deserialize` when the response body cannot
    /// be parsed into the expected shape.
    #[tracing::instrument(name = "greetings.get_definitions", skip(self))]
    pub async fn get_greeting_definitions(
        &self,
    ) -> crate::errors::Result<Vec<crate::models::GreetingDefinition>> {
        definitions::get_greeting_definitions(self).await
    }

    /// Sends a greeting of `greeting_type` to the player identified by
    /// `profile_id`.
    ///
    /// `greeting_type` must be one of the `id` values returned by
    /// [`Self::get_greeting_definitions`].  `profile_id` is the target
    /// player's profile UUID as returned by the profiles endpoint.
    ///
    /// # Errors
    ///
    /// * `MspError::Api(200, …)` — the server accepted the request but
    ///   reported a logical failure (daily cap hit, invalid type, …).
    ///   The error message includes `reason`, `next_greeting_seconds_remaining`,
    ///   and a human-readable `message` field from the server payload.
    /// * `MspError::Api(non-2xx, …)` — HTTP-level failure.
    /// * `MspError::Deserialize` — response body could not be parsed.
    #[tracing::instrument(
        name = "greetings.send",
        skip(self),
        fields(greeting_type = %greeting_type, profile_id = %profile_id)
    )]
    pub async fn send_greeting(
        &self,
        greeting_type: &str,
        profile_id:    &str,
    ) -> crate::errors::Result<crate::models::SendGreetingResult> {
        send::send_greeting(self, greeting_type, profile_id).await
    }
}