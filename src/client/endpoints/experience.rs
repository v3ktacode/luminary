// src/endpoints/experience.rs
//
// Fetches a profile's XP and level standing for this game. It's a
// read-only endpoint
// Routed to the cluster matching the target profile's region, just like
// the other per-profile endpoints (attributes, collects, quests).

use std::sync::Arc;

use serde::Deserialize;
use wreq::Client;

use super::super::http::{build_headers, decode_response_value, ContentType};
use crate::config::MspConfig;
use crate::errors::{MspError, Result};
use crate::session::SessionStore;

/// Short label attached to every `MspError` raised from this module.
const EP: &str = "experience";

const ORIGIN: &str = "https://moviestarplanet2.com";
const REFERER: &str = "https://moviestarplanet2.com/";

pub struct ExperienceEndpoint<'c> {
    pub(crate) http: &'c Client,
    pub(crate) session: &'c SessionStore,
    pub(crate) config: Arc<MspConfig>,
}

/// A profile's current XP standing within this game.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileExperience {
    pub xp: u64,
    pub level: u32,
    /// The XP threshold at which the current level began.
    #[serde(rename = "currentLevelXpMin")]
    pub current_level_xp_min: u64,
    /// The XP threshold at which the current level ends (i.e. the XP needed
    /// to reach the next level).
    #[serde(rename = "currentLevelXpMax")]
    pub current_level_xp_max: u64,
}

/// The server wraps the actual data in a single-field envelope rather than
/// returning `ProfileExperience` at the top level — this type exists purely
/// to unwrap that one layer during deserialization.
#[derive(Debug, Deserialize)]
struct ExperienceEnvelope {
    experience: ProfileExperience,
}

impl<'c> ExperienceEndpoint<'c> {
    /// Fetches XP and level information for a profile.
    ///
    /// Note this takes an explicit `profile_id` rather than defaulting to
    /// the logged-in profile — you can look up any profile's experience,
    /// not just your own. The bearer token used for the request is still
    /// always the current session's, though; the target profile only
    /// affects which XP record is returned.
    ///
    /// Uses the current session's region to pick the right server cluster,
    /// on the assumption that both profiles being queried live on the same
    /// regional cluster as the logged-in account. If MSP2 allows querying
    /// a profile registered in a different region than your own, this
    /// would need to route by the *target* profile's region instead — but
    /// there's no way to know that region ahead of the request.
    #[tracing::instrument(name = "experience.get", skip(self), fields(profile_id = %profile_id))]
    pub async fn get_experience(&self, profile_id: &str) -> Result<ProfileExperience> {
        let session = self.session.get().await?;
        let url = self.config.experience_regional(profile_id, &session.region);

        let response = self
            .http
            .get(&url)
            .headers(build_headers(
                ContentType::Json,
                Some(&session.bearer()),
                ORIGIN,
                REFERER,
            ))
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        let envelope: ExperienceEnvelope = decode_response_value(response, EP)
            .await
            .and_then(|v| serde_json::from_value(v).map_err(|e| MspError::deserialize(e, EP)))?;

        Ok(envelope.experience)
    }
}