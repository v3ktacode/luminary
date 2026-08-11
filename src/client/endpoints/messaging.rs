use serde_json::Value;
use wreq::{Client, StatusCode};

const EP: &'static str = "messaging";

use crate::{
    config::MspConfig,
    errors::{MspError, Result},
    models::{ChatMessage, Conversation, ConversationEntry, LatestMessage, MessageReceipt},
    session::SessionStore,
};
use super::super::http::{build_headers, decode_response_value, ContentType};

pub struct MessagingEndpoint<'c> {
    pub(crate) http:    &'c Client,
    pub(crate) session: &'c SessionStore,
    pub(crate) config:  std::sync::Arc<MspConfig>,
}

impl<'c> MessagingEndpoint<'c> {
    fn headers_for(&self, bearer: &str) -> wreq::header::HeaderMap {
        build_headers(
            ContentType::Json,
            Some(bearer),
            &self.config.origin,
            &self.config.referer,
        )
    }

    #[tracing::instrument(name = "messaging.find_conversation", skip(self),
        fields(other = %other_profile_id))]
    pub async fn find_conversation(
        &self,
        other_profile_id: &str,
    ) -> Result<Option<Conversation>> {
        let session = self.session.get().await?;
        let url = self.config.conversations_by_profile(
            &session.profile_id, other_profile_id,
        );

        eprintln!("[messaging] GET find_conversation: {url}");

        let response = self
            .http
            .get(&url)
            .headers(self.headers_for(&session.bearer()))
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        let status = response.status();
        eprintln!("[messaging] find_conversation status: {status}");

        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let (bytes, status) = read_raw(response, "find_conversation").await?;
        if is_empty_or_null(&bytes) {
            eprintln!("[messaging] find_conversation: empty/null body -> None");
            return Ok(None);
        }
        if !status.is_success() {
            return Err(MspError::api(EP, status.as_u16(), String::from_utf8_lossy(&bytes).into_owned()));
        }

        let value: Value = serde_json::from_slice(&bytes).map_err(|e| MspError::deserialize(e, EP))?;
        Ok(Some(parse_conversation(&value)?))
    }

    #[tracing::instrument(name = "messaging.mark_read", skip(self),
        fields(conversation_id = %conversation_id))]
    pub async fn mark_conversation_as_read(
        &self,
        conversation_id: &str,
    ) -> Result<ConversationEntry> {
        let session = self.session.get().await?;
        let url = self.config.conversation_participant(
            conversation_id, &session.profile_id,
        );

        let payload = serde_json::json!({ "numUnread": 0, "isMuted": false });

        eprintln!("[messaging] PUT mark_conversation_as_read: {url}");

        let response = self
            .http
            .put(&url)
            .headers(self.headers_for(&session.bearer()))
            .json(&payload)
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        let status = response.status();
        let bytes = response.bytes().await.map_err(|e| MspError::from_wreq(e, EP))?;

        eprintln!(
            "[messaging] mark_conversation_as_read status={status} body=\"{}\"",
            String::from_utf8_lossy(&bytes)
        );

        if !status.is_success() {
            return Err(MspError::api(EP, status.as_u16(), String::from_utf8_lossy(&bytes).into_owned()));
        }

        let raw: Value = serde_json::from_slice(&bytes).map_err(|e| MspError::deserialize(e, EP))?;
        parse_conversation_entry(raw)
    }

    #[tracing::instrument(name = "messaging.create_conversation", skip(self),
        fields(other = %other_profile_id))]
    pub async fn create_conversation(
        &self,
        other_profile_id: &str,
    ) -> Result<Conversation> {
        let session = self.session.get().await?;
        let url = self.config.conversations_create(&session.profile_id);

        let payload = serde_json::json!({
            "name":         "name",
            "message":      null,
            "type":         "OneToOne",
            "participants": [session.profile_id, other_profile_id],
        });

        eprintln!("[messaging] POST create_conversation: {url}");

        let response = self
            .http
            .post(&url)
            .headers(self.headers_for(&session.bearer()))
            .json(&payload)
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        let value = decode_response_value(response, EP).await?;
        parse_conversation(&value)
    }

    pub async fn get_or_create_conversation(
        &self,
        other_profile_id: &str,
    ) -> Result<Conversation> {
        if let Some(conv) = self.find_conversation(other_profile_id).await? {
            return Ok(conv);
        }
        self.create_conversation(other_profile_id).await
    }

    #[tracing::instrument(name = "messaging.send_message", skip(self, body),
        fields(conversation_id = %conversation_id))]
    pub async fn send_message(
        &self,
        conversation_id: &str,
        body:            &str,
    ) -> Result<MessageReceipt> {
        let session = self.session.get().await?;
        let url     = self.config.conversation_history(conversation_id);

        let payload = serde_json::json!({
            "Author":      session.profile_id,
            "MessageType": "ChatMessageV2",
            "MessageBody": body,
        });

        let response = self
            .http
            .post(&url)
            .headers(self.headers_for(&session.bearer()))
            .json(&payload)
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        let value = decode_response_value(response, EP).await?;
        parse_message_receipt(&value)
    }

    #[tracing::instrument(name = "messaging.get_conversations", skip(self))]
    pub async fn get_conversations(
        &self,
        page:      u32,
        page_size: u32,
    ) -> Result<ConversationPage> {
        let session = self.session.get().await?;

        let base_url = self.config.conversations_list(&session.profile_id);
        let separator = if base_url.contains('?') { "&" } else { "?" };
        let url = format!("{base_url}{separator}page={page}&pageSize={page_size}");

        eprintln!("[messaging] GET get_conversations (page={page}, pageSize={page_size}): {url}");

        let response = self
            .http
            .get(&url)
            .headers(self.headers_for(&session.bearer()))
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        let status = response.status();
        eprintln!("[messaging] get_conversations status: {status}");

        if status == StatusCode::NOT_FOUND {
            eprintln!("[messaging] 404 -> empty conversation page");
            return Ok(ConversationPage {
                conversations: Vec::new(),
                unread_conversation_ids: Vec::new(),
            });
        }

        let headers_dump: Vec<String> = response
            .headers()
            .iter()
            .map(|(k, v)| format!("{k}: {}", v.to_str().unwrap_or("<binary>")))
            .collect();
        eprintln!("[messaging] response headers: {headers_dump:?}");

        let bytes = response.bytes().await.map_err(|e| MspError::from_wreq(e, EP))?;

        eprintln!(
            "[messaging] get_conversations body ({} bytes): {}",
            bytes.len(),
            String::from_utf8_lossy(&bytes)
        );

        if !status.is_success() {
            return Err(MspError::api(
                EP,
                status.as_u16(),
                String::from_utf8_lossy(&bytes).into_owned(),
            ));
        }

        if is_empty_or_null(&bytes) {
            eprintln!("[messaging] empty/null body -> treating as empty conversation page");
            return Ok(ConversationPage {
                conversations: Vec::new(),
                unread_conversation_ids: Vec::new(),
            });
        }

        let raw: Value = serde_json::from_slice(&bytes).map_err(|e| {
            eprintln!(
                "[messaging] FAILED to deserialize get_conversations body: {e}\nraw body: {}",
                String::from_utf8_lossy(&bytes)
            );
            MspError::deserialize(e, EP)
        })?;

        let entries_raw = raw.as_array().ok_or_else(|| {
            eprintln!(
                "[messaging] get_conversations body was valid JSON but not an array: {raw}"
            );
            MspError::api(
                EP, 200,
                "Expected a JSON array from conversations list endpoint",
            )
        })?;

        let mut conversations           = Vec::with_capacity(entries_raw.len());
        let mut unread_conversation_ids = Vec::new();

        for entry in entries_raw {
            let conv = parse_conversation_entry(entry.clone())?;
            if conv.number_of_unread_messages > 0 {
                unread_conversation_ids.push(conv.conversation_id.clone());
            }
            conversations.push(conv);
        }

        eprintln!(
            "[messaging] get_conversations page={page}: {} entries, {} unread",
            conversations.len(),
            unread_conversation_ids.len()
        );

        Ok(ConversationPage { conversations, unread_conversation_ids })
    }

    #[tracing::instrument(name = "messaging.get_chat_history", skip(self),
        fields(conversation_id = %conversation_id))]
    pub async fn get_chat_history(
        &self,
        conversation_id: &str,
        page_size:       u32,
    ) -> Result<Vec<ChatMessage>> {
        let session = self.session.get().await?;
        let url = format!(
            "{}&profileId={}&pageSize={page_size}",
            self.config.conversation_history(conversation_id),
            session.profile_id,
        );

        eprintln!("[messaging] GET get_chat_history: {url}");

        let response = self
            .http
            .get(&url)
            .headers(self.headers_for(&session.bearer()))
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        let status = response.status();
        let bytes = response.bytes().await.map_err(|e| MspError::from_wreq(e, EP))?;

        eprintln!(
            "[messaging] get_chat_history status={status} body ({} bytes): {}",
            bytes.len(),
            String::from_utf8_lossy(&bytes)
        );

        if !status.is_success() {
            return Err(MspError::api(EP, status.as_u16(), String::from_utf8_lossy(&bytes).into_owned()));
        }

        if is_empty_or_null(&bytes) {
            return Ok(Vec::new());
        }

        let raw: Value = serde_json::from_slice(&bytes).map_err(|e| MspError::deserialize(e, EP))?;
        serde_json::from_value(raw).map_err(|e| MspError::deserialize(e, EP))
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConversationPage {
    pub conversations:            Vec<ConversationEntry>,
    pub unread_conversation_ids:  Vec<String>,
}

/// Returns true if the response body is empty or the literal JSON `null`.
fn is_empty_or_null(bytes: &[u8]) -> bool {
    let trimmed = std::str::from_utf8(bytes).unwrap_or("").trim();
    trimmed.is_empty() || trimmed == "null"
}

/// Reads status + raw bytes from a response without attempting to parse it.
async fn read_raw(response: wreq::Response, ctx: &str) -> Result<(Vec<u8>, StatusCode)> {
    let status = response.status();
    let raw = response.bytes().await.map_err(|e| MspError::from_wreq(e, EP))?;
    let bytes: Vec<u8> = raw.to_vec();
    eprintln!(
        "[messaging] {ctx} status={status} body ({} bytes): {}",
        bytes.len(),
        String::from_utf8_lossy(&bytes)
    );
    Ok((bytes, status))
}

fn parse_conversation(data: &Value) -> Result<Conversation> {
    let conversation_id = data["conversationId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| MspError::api(EP, 422, "Missing 'conversationId' in conversation response"))?
        .to_owned();

    let conversation_name = data["conversationName"]
        .as_str()
        .unwrap_or_default()
        .to_owned();

    Ok(Conversation { conversation_id, conversation_name })
}

fn parse_message_receipt(data: &Value) -> Result<MessageReceipt> {
    let conversation_id = data["conversationId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| MspError::api(EP, 422, "Missing 'conversationId' in message receipt"))?
        .to_owned();

    let sender_profile_id = data["senderProfileId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| MspError::api(EP, 422, "Missing 'senderProfileId' in message receipt"))?
        .to_owned();

    let message_body = data["messageBody"]
        .as_str()
        .unwrap_or_default()
        .to_owned();

    let timestamp = data["timestamp"]
        .as_str()
        .unwrap_or_default()
        .to_owned();

    Ok(MessageReceipt { conversation_id, message_body, sender_profile_id, timestamp })
}

fn parse_conversation_entry(mut raw: Value) -> Result<ConversationEntry> {
    // FIX: Inject dummy participants array if server omits it
    if let Some(obj) = raw.as_object_mut() {
        if !obj.contains_key("participants") {
            obj.insert("participants".to_string(), serde_json::json!(Vec::<String>::new()));
        }
    }

    let mut entry: ConversationEntry = serde_json::from_value(raw)
        .map_err(|e| MspError::deserialize(e, EP))?;

    if let Some(ref raw_str) = entry.latest_message {
        match serde_json::from_str::<LatestMessage>(raw_str) {
            Ok(parsed) => entry.latest_message_parsed = Some(parsed),
            Err(e) => {
                tracing::debug!(
                    "Could not parse latest_message JSON: {e}. Field will be None."
                );
            }
        }
    }

    Ok(entry)
}