use wreq::header::{
    HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE,
    ORIGIN as ORIGIN_HDR, REFERER as REFERER_HDR,
};
use wreq::Response;
use serde::de::DeserializeOwned;
use serde_json::Value;
use crate::errors::{MspError, Result};

const APPLICATION_JSON: &str = "application/json";
const FORM_URLENCODED:  &str = "application/x-www-form-urlencoded";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType { Form, Json }

impl ContentType {
    #[inline]
    const fn as_str(self) -> &'static str {
        match self {
            Self::Form => FORM_URLENCODED,
            Self::Json => APPLICATION_JSON,
        }
    }
}

#[must_use]
pub fn build_headers(kind: ContentType, bearer: Option<&str>, origin: &str, referer: &str) -> HeaderMap {
    let mut h = HeaderMap::with_capacity(5);
    h.insert(CONTENT_TYPE, HeaderValue::from_static(kind.as_str()));
    h.insert(ACCEPT,       HeaderValue::from_static(APPLICATION_JSON));

    match HeaderValue::from_str(referer) {
        Ok(v) => { h.insert(REFERER_HDR, v); }
        Err(e) => tracing::warn!("Skipping Referer header — invalid value: {e}"),
    }
    if kind == ContentType::Form {
        match HeaderValue::from_str(origin) {
            Ok(v) => { h.insert(ORIGIN_HDR, v); }
            Err(e) => tracing::warn!("Skipping Origin header — invalid value: {e}"),
        }
    }
    if let Some(token) = bearer {
        match HeaderValue::from_str(token) {
            Ok(mut v) => { v.set_sensitive(true); h.insert(AUTHORIZATION, v); }
            Err(e) => tracing::warn!(
                "Skipping Authorization header — token is not a valid header value: {e}"
            ),
        }
    }
    h
}

pub async fn decode_response<T: DeserializeOwned>(response: Response, endpoint: &'static str) -> Result<T> {
    let status = response.status();

    if status.as_u16() == 429 {
        let retry_after_secs = response.headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60);
        return Err(MspError::RateLimited { endpoint, retry_after_secs });
    }

    if !status.is_success() {
        // Even on a non-2xx response, the body is often structured JSON
        // (`{"error": ..., "error_description": ...}`, `{"message": "..."}`,
        // etc). Try to pull a clean code/message out of it instead of always
        // surfacing the raw body text.
        let bytes = response.bytes().await.unwrap_or_default();
        return Err(api_error_from_body(endpoint, status.as_u16(), &bytes));
    }

    let bytes = response.bytes().await.map_err(|e| MspError::from_wreq(e, endpoint))?;
    let value = serde_json::from_slice::<Value>(&bytes).map_err(|e| MspError::deserialize(e, endpoint))?;
    let value = ensure_no_api_error(value, endpoint)?;
    serde_json::from_value(value).map_err(|e| MspError::deserialize(e, endpoint))
}

pub async fn decode_response_value(response: Response, endpoint: &'static str) -> Result<Value> {
    decode_response::<Value>(response, endpoint).await
}

/// Builds an `MspError::Api` from a non-2xx response body, using the real
/// HTTP status code. Falls back to the raw body text when it isn't JSON, or
/// isn't in a shape we recognize.
fn api_error_from_body(endpoint: &'static str, status: u16, bytes: &[u8]) -> MspError {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return MspError::api(endpoint, status, String::from_utf8_lossy(bytes).into_owned());
    };

    match extract_error_fields(&value) {
        Some((code, message)) => MspError::Api { endpoint, status, code, message },
        None => MspError::api(endpoint, status, value.to_string()),
    }
}

/// Some endpoints (GraphQL-style) return HTTP 200 with an `errors`/`error`
/// payload describing the actual failure. Catch that here before the caller
/// tries to deserialize the body into its expected success type.
pub(crate) fn ensure_no_api_error(value: Value, endpoint: &'static str) -> Result<Value> {
    if let Some(errors) = value.get("errors") {
        if let Some(arr) = errors.as_array().filter(|a| !a.is_empty()) {
            let joined = arr.iter()
                .filter_map(|e| e.get("message").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("; ");
            let message = if joined.is_empty() {
                "unknown GraphQL error".to_string()
            } else {
                joined
            };
            return Err(MspError::graphql(endpoint, message));
        }
    }

    if value.get("error").is_some_and(|v| !v.is_null()) {
        // No real HTTP status to draw on here (this is a 2xx response), so
        // fall back to whatever the body itself claims, defaulting to 400.
        let status = value.get("statusCode").or_else(|| value.get("status"))
            .and_then(Value::as_u64).map(|s| s as u16).unwrap_or(400);
        let (code, message) = extract_error_fields(&value)
            .unwrap_or_else(|| (None, "unknown error".to_string()));
        return Err(MspError::Api { endpoint, status, code, message });
    }

    Ok(value)
}

/// Best-effort extraction of an `(code, message)` pair from a JSON error
/// body. Recognizes a few common shapes:
/// - `{"error": "some_code", "error_description": "human readable"}`
/// - `{"error": {"code": "...", "message": "..."}}`
/// - `{"message": "human readable"}`
fn extract_error_fields(value: &Value) -> Option<(Option<String>, String)> {
    if let Some(err) = value.get("error").filter(|v| !v.is_null()) {
        return Some(match err {
            Value::String(code) => {
                let message = match value.get("error_description").and_then(Value::as_str) {
                    Some(desc) => format!("{code}: {desc}"),
                    None => code.clone(),
                };
                (Some(code.clone()), message)
            }
            Value::Object(_) => {
                let code = err.get("code").and_then(Value::as_str).map(str::to_owned);
                let message = err.get("message").and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| err.to_string());
                (code, message)
            }
            other => (None, other.to_string()),
        });
    }

    if let Some(message) = value.get("message").and_then(Value::as_str) {
        return Some((None, message.to_owned()));
    }

    None
}

/// Note: the response itself was already a 2xx by the time this runs (it's
/// only ever called after `decode_response` succeeds), so there's no real
/// HTTP status to attach to a missing-field error. 422 (Unprocessable
/// Entity) is used here as a synthetic status purely to signal "the shape
/// of this JSON wasn't what we expected."
#[inline]
pub fn required_str<'v>(value: &'v Value, field: &str, endpoint: &'static str) -> Result<&'v str> {
    value.get(field).and_then(Value::as_str)
        .ok_or_else(|| MspError::api(endpoint, 422,
            format!("Missing or invalid field '{field}' in response")))
}

#[inline]
#[must_use]
pub fn optional_str<'v>(value: &'v Value, field: &str) -> Option<&'v str> {
    value.get(field).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err_msg(v: Value) -> String {
        api_error_from_body("test", 400, serde_json::to_vec(&v).unwrap().as_slice()).to_string()
    }

    #[test]
    fn extracts_string_error_with_description() {
        let v = serde_json::json!({"error": "invalid_grant", "error_description": "Token expired"});
        assert!(err_msg(v).contains("invalid_grant: Token expired"));
    }

    #[test]
    fn extracts_object_error() {
        let v = serde_json::json!({"error": {"code": "E42", "message": "Bad thing happened"}});
        let msg = err_msg(v);
        assert!(msg.contains("Bad thing happened"));
        assert!(msg.contains("E42"));
    }

    #[test]
    fn extracts_top_level_message() {
        let v = serde_json::json!({"message": "Not authorized"});
        assert!(err_msg(v).contains("Not authorized"));
    }

    #[test]
    fn falls_back_to_raw_body_when_unstructured() {
        let v = serde_json::json!({"foo": "bar"});
        // No recognizable error shape — should still produce *some* message,
        // not panic, and should include the real HTTP status.
        assert!(err_msg(v).contains("400"));
    }

    #[test]
    fn non_json_body_falls_back_to_text() {
        let msg = api_error_from_body("test", 502, b"<html>Bad Gateway</html>").to_string();
        assert!(msg.contains("502"));
    }

    #[test]
    fn graphql_errors_without_message_field_get_placeholder() {
        let v = serde_json::json!({"errors": [{"path": ["foo"]}]});
        let err = ensure_no_api_error(v, "test").unwrap_err();
        assert!(err.to_string().contains("unknown GraphQL error"));
    }

    #[test]
    fn graphql_errors_with_message_are_joined() {
        let v = serde_json::json!({"errors": [{"message": "a"}, {"message": "b"}]});
        let err = ensure_no_api_error(v, "test").unwrap_err();
        let s = err.to_string();
        assert!(s.contains("a; b"));
    }
}