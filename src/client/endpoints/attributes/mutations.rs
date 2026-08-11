// src/client/endpoints/attributes/mutations.rs
//
// Named convenience wrappers around `update_additional_data_key`, one per
// well-known key inside `additionalData`. There are certainly more such
// keys in the wild (the field is a free-form bag), but these are the only
// ones this crate exposes dedicated methods for — anything else should go
// through `update_additional_data_key` directly.

use serde_json::Value;

use super::{parsing, AttributesEndpoint, EP};
use crate::errors::{MspError, Result};
use crate::models::ProfileAttributes;

const MOOD_KEY: &str = "Mood";
const GENDER_KEY: &str = "Gender";
const WAYD_KEY: &str = "WAYD"; // "What Are You Doing" status ID

impl<'c> AttributesEndpoint<'c> {
    /// Sets the profile's mood text.
    pub async fn set_mood(&self, mood: &str) -> Result<ProfileAttributes> {
        self.update_additional_data_key(MOOD_KEY, mood).await
    }

    /// Flips the profile's gender between `"Boy"` and `"Girl"`.
    ///
    /// Reads the current value first, so this only works if the current
    /// gender is one of those two known values — anything else (missing,
    /// null, or some other string) is treated as an error rather than
    /// guessed at.
    #[tracing::instrument(name = "attributes.gender_swap", skip(self))]
    pub async fn gender_swap(&self) -> Result<ProfileAttributes> {
        let (bearer, url) = self.own_bearer_and_url().await?;

        let mut attributes = self.get_json(&url, &bearer).await?;
        let current = attributes
            .get("additionalData")
            .and_then(|d| d.get(GENDER_KEY))
            .and_then(Value::as_str);

        let swapped = match current {
            Some("Girl") => "Boy",
            Some("Boy") => "Girl",
            other => {
                return Err(MspError::api(
                    EP,
                    422,
                    format!("Cannot swap '{GENDER_KEY}' — unexpected value: {other:?}"),
                ));
            }
        };

        parsing::set_additional_data(&mut attributes, GENDER_KEY, Value::from(swapped));

        let updated = self.put_json(&url, &bearer, &attributes).await?;
        parsing::parse_attributes(&updated)
    }

    /// Sets the profile's WAYD ("What Are You Doing") status id.
    pub async fn update_wayd_id(&self, wayd_id: &str) -> Result<ProfileAttributes> {
        self.update_additional_data_key(WAYD_KEY, wayd_id).await
    }
}