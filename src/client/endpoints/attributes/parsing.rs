// src/client/endpoints/attributes/parsing.rs
//
// JSON shaping: turning a raw attributes response into `ProfileAttributes`,
// and patching a single key into the `additionalData` object before sending
// an update back.

use serde_json::Value;

use super::EP;
use crate::errors::{MspError, Result};
use crate::models::ProfileAttributes;

/// Inserts or overwrites a single key inside `attributes.additionalData`,
/// creating the `additionalData` object if it doesn't exist yet or isn't
/// currently an object (e.g. if the server returned `null`).
pub(super) fn set_additional_data(attributes: &mut Value, key: &str, value: Value) {
    let existing = attributes
        .get_mut("additionalData")
        .filter(|v| v.is_object())
        .and_then(Value::as_object_mut);

    match existing {
        Some(obj) => {
            obj.insert(key.to_owned(), value);
        }
        None => {
            let mut map = serde_json::Map::new();
            map.insert(key.to_owned(), value);
            attributes["additionalData"] = Value::Object(map);
        }
    }
}

pub(super) fn wreq_err(e: wreq::Error) -> MspError {
    MspError::from_wreq(e, EP)
}

/// Pulls a required, non-empty string field out of a JSON object.
///
/// Note: indexing a `serde_json::Value` with a missing key returns `Value::Null`
/// rather than panicking, so this is safe to call even when `field` isn't
/// present at all.
fn required_field<'v>(value: &'v Value, field: &str) -> Result<&'v str> {
    value[field]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            MspError::api(EP, 422, format!("Missing '{field}' in attributes response"))
        })
}

/// Parses a raw attributes JSON response into `ProfileAttributes`.
///
/// `profileId` and `gameId` are treated as required — if either is missing,
/// something is wrong enough with the response that we'd rather fail loudly
/// than hand back a half-populated struct. `avatarId` and `additionalData`
/// are optional and default to an empty string / `null` respectively, since
/// not every profile has them set.
pub(super) fn parse_attributes(value: &Value) -> Result<ProfileAttributes> {
    Ok(ProfileAttributes {
        profile_id: required_field(value, "profileId")?.to_owned(),
        game_id: required_field(value, "gameId")?.to_owned(),
        avatar_id: value["avatarId"].as_str().unwrap_or_default().to_owned(),
        additional_data: value.get("additionalData").cloned().unwrap_or(Value::Null),
    })
}