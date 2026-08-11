//! Pets endpoint.

use std::sync::Arc;
use serde::{Deserialize, Serialize};
use wreq::Client;

use crate::config::MspConfig;
use crate::errors::{MspError, Result};
use crate::session::SessionStore;
use super::super::http::{build_headers, ContentType};

const EP: &'static str = "pets";
const GAME_ID: &'static str = "j68d";

#[derive(Debug, Serialize)]
struct InteractionRequest {
    #[serde(rename = "profileId")]
    profile_id: String,
    #[serde(rename = "gameId")]
    game_id: String,
}

#[derive(Debug, Deserialize)]
pub struct InteractionResponse {
    pub interactions: u32,
    #[serde(rename = "maxRewardedInteractions")]
    pub max_rewarded_interactions: u32,
    #[serde(rename = "secondsToReset")]
    pub seconds_to_reset: u32,
}

pub struct PetsEndpoint<'c> {
    pub(crate) http:    &'c Client,
    pub(crate) session: &'c SessionStore,
    pub(crate) config:  Arc<MspConfig>,
}

impl<'c> PetsEndpoint<'c> {
    fn headers(&self, bearer: &str) -> wreq::header::HeaderMap {
        build_headers(
            ContentType::Json,
            Some(bearer),
            &self.config.origin,
            &self.config.referer,
        )
    }

    /// Interagit avec un animal (pet) pour progresser dans la quête `daily_pet_pets`.
    ///
    /// # Arguments
    /// * `pet_id` - L'identifiant de l'animal (ex: "f922447a43434c1f9e65ebdae0f3c194")
    ///
    /// # Example
    /// ```no_run
    /// let response = client.pets().interact("f922447a43434c1f9e65ebdae0f3c194").await?;
    /// println!("Interactions: {}/{}", response.interactions, response.max_rewarded_interactions);
    /// ```
    #[tracing::instrument(name = "pets.interact", skip(self), fields(pet_id = %pet_id))]
    pub async fn interact(&self, pet_id: &str) -> Result<InteractionResponse> {
        let session = self.session.get().await?;
        let url = format!(
            "{}/pets/v1/pets/{}/interactions",
            self.config.base_url(),
            pet_id
        );

        let body = InteractionRequest {
            profile_id: session.profile_id.clone(),
            game_id: GAME_ID.to_string(),
        };

        let response = self
            .http
            .post(&url)
            .headers(self.headers(&session.bearer()))
            .json(&body)
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(MspError::api(EP, status.as_u16(), body));
        }

        let bytes = response.bytes().await.map_err(|e| MspError::from_wreq(e, EP))?;
        serde_json::from_slice(&bytes).map_err(|e| MspError::deserialize(e, EP))
    }
}