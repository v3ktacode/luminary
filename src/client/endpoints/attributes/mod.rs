// src/client/endpoints/attributes/mod.rs
//
// Handles a profile's "attributes" — a small JSON blob the server keeps per
// profile, covering things like the avatar ID, gender, current mood, and a
// free-form `additionalData` object used for miscellaneous key/value state
// (WAYD id, gender, and whatever else the game stores there).
//
// There's no PATCH endpoint for this resource — every update is a full
// GET-modify-PUT round trip: fetch the current attributes, change the one
// field we care about, then send the whole object back. This means two
// concurrent updates to the same profile can race and one of them will
// silently clobber the other. Worth keeping in mind if you're calling these
// methods from multiple tasks for the same profile at once.
//
// This module is split into three files:
//
//   mod.rs         — the endpoint struct, the two "generic" operations
//                    (`get`, `update_additional_data_key`), and the shared
//                    HTTP plumbing every operation goes through.
//   mutations.rs   — the named convenience wrappers (`set_mood`,
//                    `gender_swap`, `update_wayd_id`) built on top of
//                    `update_additional_data_key`.
//   parsing.rs     — turning raw JSON into `ProfileAttributes` and back.

mod mutations;
mod parsing;

use std::sync::Arc;

use serde_json::Value;
use wreq::Client;

use super::super::http::{build_headers, decode_response_value, ContentType};
use crate::config::MspConfig;
use crate::errors::Result;
use crate::models::ProfileAttributes;
use crate::session::SessionStore;

/// Short label attached to every `MspError` raised from this module.
pub(super) const EP: &str = "attributes";

pub struct AttributesEndpoint<'c> {
    pub(crate) http: &'c Client,
    pub(crate) session: &'c SessionStore,
    pub(crate) config: Arc<MspConfig>,
}

impl<'c> AttributesEndpoint<'c> {
    /// Fetches the attributes for a profile.
    ///
    /// Pass `None` to fetch the currently logged-in profile's own
    /// attributes, or `Some(id)` to look up any other profile.
    #[tracing::instrument(name = "attributes.get", skip(self))]
    pub async fn get(&self, profile_id: Option<&str>) -> Result<ProfileAttributes> {
        let session = self.session.get().await?;
        let target_id = profile_id.unwrap_or(&session.profile_id);
        let url = self.config.attributes(target_id);

        let value = self.get_json(&url, &session.bearer()).await?;
        parsing::parse_attributes(&value)
    }

    /// Sets a single key inside the profile's `additionalData` object and
    /// returns the resulting attributes.
    ///
    /// This is a read-modify-write operation: it fetches the current
    /// attributes, patches just this one key locally, and PUTs the whole
    /// object back. Only works on your own profile.
    ///
    /// If you need to set a key this crate doesn't have a named helper for
    /// (see `mutations.rs` for `set_mood`, `gender_swap`, `update_wayd_id`),
    /// this is the one to use directly.
    #[tracing::instrument(name = "attributes.update_key", skip(self, value), fields(key = %key))]
    pub async fn update_additional_data_key(
        &self,
        key: &str,
        value: impl Into<Value>,
    ) -> Result<ProfileAttributes> {
        let (bearer, url) = self.own_bearer_and_url().await?;

        let mut attributes = self.get_json(&url, &bearer).await?;
        parsing::set_additional_data(&mut attributes, key, value.into());

        let updated = self.put_json(&url, &bearer, &attributes).await?;
        parsing::parse_attributes(&updated)
    }

    // -------------------------------------------------------------------
    // Internal helpers
    //
    // Marked `pub(super)` rather than private so `mutations.rs` — which
    // implements more methods on this same struct in a separate file —
    // can reuse them instead of duplicating the request logic.
    // -------------------------------------------------------------------

    /// Builds the standard JSON + bearer header set used by every request
    /// in this endpoint.
    pub(super) fn headers(&self, bearer: &str) -> wreq::header::HeaderMap {
        build_headers(
            ContentType::Json,
            Some(bearer),
            &self.config.origin,
            &self.config.referer,
        )
    }

    /// Convenience shortcut for the methods that only ever operate on the
    /// logged-in user's own profile (everything except `get`, which also
    /// accepts an arbitrary profile id).
    pub(super) async fn own_bearer_and_url(&self) -> Result<(String, String)> {
        let session = self.session.get().await?;
        let bearer = session.bearer();
        let url = self.config.attributes(&session.profile_id);
        Ok((bearer, url))
    }

    pub(super) async fn get_json(&self, url: &str, bearer: &str) -> Result<Value> {
        let response = self
            .http
            .get(url)
            .headers(self.headers(bearer))
            .send()
            .await
            .map_err(parsing::wreq_err)?;
        decode_response_value(response, EP).await
    }

    /// Sends the full attributes object back to the server. Note this is a
    /// full replace, not a partial patch — the entire `attributes` value
    /// passed in becomes the new server-side state.
    pub(super) async fn put_json(&self, url: &str, bearer: &str, body: &Value) -> Result<Value> {
        let response = self
            .http
            .put(url)
            .headers(self.headers(bearer))
            .json(body)
            .send()
            .await
            .map_err(parsing::wreq_err)?;
        decode_response_value(response, EP).await
    }
}