use serde_json::Value;
use wreq::Client;
use bson;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use crate::{
    errors::{MspError, Result},
    models::{Ugc, ProfileUgcsResult, ProfileUgcNode},
    session::SessionStore,
};
use super::super::http::{build_headers, ContentType};

const EP: &'static str = "ugcs";

const UGC_GRAPHQL_ENDPOINT: &str         = "https://eu.mspapis.com/edgeugc/graphql";
const COMMENTS_GRAPHQL_ENDPOINT: &str    = "https://eu.mspapis.com/edgecomments/graphql";
const FEDERATION_GRAPHQL_ENDPOINT: &str  = "https://eu.mspapis.com/federationgateway/graphql";
const UGC_CDN_BASE: &str = "https://ugc-eu.mspcdns.com/";
const PGC_BASE: &str     = "https://eu.mspapis.com/profilegeneratedcontent/v2";

// Clé HMAC identique à celle utilisée côté client web
const HMAC_KEY_STR: &str = "WaENqVS5ziQSAVEUtvXU5qzgDzS/d0DdQZK5V6U7kL8=";

const GET_COMMENTS_COUNT_QUERY: &str = "\
query GetCommentsCount($entityType: EntityType!, $threadId: ID!) {\
  count(entityType: $entityType, threadId: $threadId) {\
    count\
  }\
}";

const GET_UGC_BY_ID_QUERY: &str = "\
query GetUgcById($ugcId: String!, $gameId: String!) {\
  ugc(input:{ugcId: $ugcId}) {\
    id title lastEditedDate lifecycleStatus privacyStatus owner type commentCount \
    ...on Movie { duration views } \
    reactions { reactionTypeId count } \
    resources { type id } \
    profile {\
      id name \
      membership { lastTierExpiry } \
      avatar(preferredGameId: $gameId) { gameId }\
    }\
  }\
}";



/// Identifies the type of UGC to fetch from a user's profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UgcType {
    Looks,
    Movies,
    Artbooks,
}

impl UgcType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Looks    => "LOOKS",
            Self::Movies   => "MOVIES",
            Self::Artbooks => "ARTBOOKS",
        }
    }
}

pub struct UgcsEndpoint<'c> {
    pub(crate) http:    &'c Client,
    pub(crate) session: &'c SessionStore,
}

impl<'c> UgcsEndpoint<'c> {
    fn headers(&self, bearer: &str) -> wreq::header::HeaderMap {
        build_headers(
            ContentType::Json,
            Some(bearer),
            "https://moviestarplanet2.com",
            "https://moviestarplanet2.com/",
        )
    }

    // ── get_status_text ───────────────────────────────────────────────────

    #[tracing::instrument(name = "ugcs.get_status_text", skip(self),
        fields(wayd_id = %wayd_id))]
    pub async fn get_status_text(&self, wayd_id: &str) -> Result<Option<String>> {
        let ugc = self
            .get_ugc_by_id(wayd_id)
            .await?
            .ok_or_else(|| MspError::api(
                EP, 404,
                format!("UGC '{wayd_id}' not found"),
            ))?;

        let resource_id = ugc
            .resources
            .iter()
            .find(|r| r.resource_type == "PgcV1")
            .map(|r| r.id.as_str())
            .ok_or_else(|| MspError::api(
                EP, 200,
                "No PgcV1 resource found in UGC",
            ))?;

        let cdn_url = format!("{UGC_CDN_BASE}{resource_id}");

        let response = self
            .http
            .get(&cdn_url)
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        if !response.status().is_success() {
            return Err(MspError::api(
                EP, response.status().as_u16(),
                format!("CDN request failed for resource '{resource_id}'"),
            ));
        }

        let bytes = response.bytes().await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        let doc = bson::Document::from_reader(&mut bytes.as_ref())
            .map_err(|e| MspError::api(
                EP, 200,
                format!("BSON decode failed: {e}"),
            ))?;

        let text = doc
            .get_array("Texts")
            .ok()
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .map(str::to_owned);

        Ok(text)
    }

    // ── set_status_text ───────────────────────────────────────────────────

    /// Met à jour le texte du statut (WAYD) du profil connecté.
    ///
    /// `wayd_id` doit être récupéré au préalable via
    /// `client.attributes().get(None)` (champ `additional_data.WAYD`).
    ///
    /// Flow :
    /// 1. Fetch les métadonnées UGC pour trouver la ressource `PgcV1`
    /// 2. Télécharge le BSON depuis le CDN
    /// 3. Modifie `Texts[0]` avec le nouveau texte
    /// 4. Sérialise en BSON, signe avec HMAC-SHA256, upload en PUT
    #[tracing::instrument(name = "ugcs.set_status_text", skip(self),
        fields(wayd_id = %wayd_id, text = %text))]
    pub async fn set_status_text(&self, wayd_id: &str, text: &str) -> Result<()> {
        let session    = self.session.get().await?;
        let bearer     = session.bearer();
        let profile_id = session.profile_id.clone();
        let game_id    = "j68d";

        // ── 1. Fetch les métadonnées UGC ──────────────────────────────────────
        let ugc = self
            .get_ugc_by_id(wayd_id)
            .await?
            .ok_or_else(|| MspError::api(EP, 404, format!("UGC '{wayd_id}' not found")))?;

        let resource_id = ugc
            .resources
            .iter()
            .find(|r| r.resource_type == "PgcV1")
            .map(|r| r.id.clone())
            .ok_or_else(|| MspError::api(EP, 200, "No PgcV1 resource found in UGC"))?;

        let title          = ugc.title.clone().unwrap_or_else(|| ugc.id.clone());
        let privacy_status = ugc.privacy_status.clone();

        tracing::debug!(resource_id = %resource_id, "set_status_text: got PgcV1 resource");

        // ── 2. Télécharge le BSON depuis le CDN ───────────────────────────────
        let cdn_url = format!("{UGC_CDN_BASE}{resource_id}");

        let cdn_resp = self
            .http
            .get(&cdn_url)
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        if !cdn_resp.status().is_success() {
            return Err(MspError::api(
                EP,
                cdn_resp.status().as_u16(),
                format!("CDN fetch failed for resource '{resource_id}'"),
            ));
        }

        let cdn_bytes = cdn_resp
            .bytes()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        // ── 3. Désérialise le BSON, modifie Texts[0] ──────────────────────────
        let mut doc = bson::Document::from_reader(&mut cdn_bytes.as_ref())
            .map_err(|e| MspError::api(EP, 200, format!("BSON decode failed: {e}")))?;

        let new_text = bson::Bson::String(text.to_owned());
        match doc.get_array_mut("Texts") {
            Ok(arr) => {
                if arr.is_empty() {
                    arr.push(new_text);
                } else {
                    arr[0] = new_text;
                }
            }
            Err(_) => {
                doc.insert("Texts", bson::Bson::Array(vec![new_text]));
            }
        }

        // ── 4. Sérialise le BSON modifié ──────────────────────────────────────
        let mut bson_bytes: Vec<u8> = Vec::new();
        doc.to_writer(&mut bson_bytes)
            .map_err(|e| MspError::api(EP, 200, format!("BSON encode failed: {e}")))?;

        // ── 5. Construit le body multipart (équivalent We() en JS) ────────────
        let body_bytes = build_pgc_body(&bson_bytes, &title, "WAYD", &privacy_status)
            .map_err(|e| MspError::api(EP, 200, format!("Body build failed: {e}")))?;

         // ── 6. Génère la signature HMAC-SHA256 (équivalent qe() en JS) ────────
        let signature = compute_signature(&body_bytes)
            .map_err(|e| MspError::api(EP, 200, format!("Signature failed: {e}")))?;

        tracing::debug!(signature = %signature, "set_status_text: computed signature");

        // ── 7. PUT vers l'API ─────────────────────────────────────────────────
        // Le segment "content/{id}" attend le WAYD id, pas le resource_id du CDN.
        let put_url = format!(
            "{PGC_BASE}/profiles/{profile_id}/games/{game_id}/content/{wayd_id}"
        );

        let mut put_headers = wreq::header::HeaderMap::new();
        put_headers.insert(
            wreq::header::AUTHORIZATION,
            {
                let mut v = wreq::header::HeaderValue::from_str(&bearer)
                    .map_err(|_| MspError::api(EP, 500, "Invalid bearer token"))?;
                v.set_sensitive(true);
                v
            },
        );
        put_headers.insert(
            wreq::header::CONTENT_TYPE,
            wreq::header::HeaderValue::from_static("application/bson"),
        );
        put_headers.insert(
            "signature",
            wreq::header::HeaderValue::from_str(&signature)
                .map_err(|_| MspError::api(EP, 500, "Invalid signature header value"))?,
        );

        let put_resp = self
            .http
            .put(&put_url)
            .headers(put_headers)
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        if !put_resp.status().is_success() {
            let status = put_resp.status().as_u16();
            let body   = put_resp.text().await.unwrap_or_default();
            return Err(MspError::api(
                EP, status,
                format!("Upload failed: {body}"),
            ));
        }

        tracing::info!(wayd_id = %wayd_id, "set_status_text: status updated successfully");
        Ok(())
    }

    // ── get_comments_count ────────────────────────────────────────────────

    #[tracing::instrument(name = "ugcs.get_comments_count", skip(self),
        fields(ugc_id = %ugc_id))]
    pub async fn get_comments_count(&self, ugc_id: &str) -> Result<u64> {
        let session = self.session.get().await?;

        let payload = serde_json::json!({
            "query":     GET_COMMENTS_COUNT_QUERY,
            "variables": serde_json::json!({
                "entityType": "UGC",
                "threadId":   ugc_id,
            }).to_string(),
        });

        let response = self
            .http
            .post(COMMENTS_GRAPHQL_ENDPOINT)
            .headers(self.headers(&session.bearer()))
            .json(&payload)
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(MspError::api(EP, status.as_u16(), body));
        }

        let bytes = response.bytes().await
            .map_err(|e| MspError::from_wreq(e, EP))?;
        let response: Value = serde_json::from_slice(&bytes)
            .map_err(|e| MspError::deserialize(e, EP))?;

        if let Some(errors) = response.get("errors") {
            return Err(MspError::graphql(EP, errors.to_string()));
        }

        let count = response["data"]["count"]["count"]
            .as_u64()
            .ok_or_else(|| MspError::api(
                EP, 200,
                "Missing 'count' field in GetCommentsCount response",
            ))?;

        Ok(count)
    }

    // ── get_ugc_by_id ─────────────────────────────────────────────────────

    #[tracing::instrument(name = "ugcs.get_by_id", skip(self),
        fields(ugc_id = %ugc_id))]
    pub async fn get_ugc_by_id(&self, ugc_id: &str) -> Result<Option<Ugc>> {
        let session = self.session.get().await?;

        let payload = serde_json::json!({
            "query":     GET_UGC_BY_ID_QUERY,
            "variables": serde_json::json!({
                "ugcId":  ugc_id,
                "gameId": "j68d",
            }).to_string(),
        });

        let response = self
            .http
            .post(UGC_GRAPHQL_ENDPOINT)
            .headers(self.headers(&session.bearer()))
            .json(&payload)
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(MspError::api(EP, status.as_u16(), body));
        }

        let bytes = response.bytes().await
            .map_err(|e| MspError::from_wreq(e, EP))?;
        let response: Value = serde_json::from_slice(&bytes)
            .map_err(|e| MspError::deserialize(e, EP))?;

        if let Some(errors) = response.get("errors") {
            return Err(MspError::graphql(EP, errors.to_string()));
        }

        let raw = &response["data"]["ugc"];

        tracing::debug!(ugc_id = %ugc_id, raw = %raw, "get_ugc_by_id raw JSON");

        if raw.is_null() {
            return Ok(None);
        }

        let ugc: Ugc = serde_json::from_value(raw.clone())
            .map_err(|e| {
                tracing::error!(
                    ugc_id = %ugc_id,
                    raw = %raw,
                    error = %e,
                    "Failed to deserialize Ugc from GraphQL response"
                );
                MspError::deserialize(e, EP)
            })?;

        Ok(Some(ugc))
    }

    // ── get_users_ugcs ────────────────────────────────────────────────────

    /// Fetches **all** UGCs of a given type for `profile_id`, automatically
    /// following pagination cursors until every page has been retrieved.
    ///
    /// # Arguments
    /// * `profile_id` – The profile identifier, e.g. `"FR|27011247"`.
    /// * `ugc_type`   – The kind of content to retrieve (`Looks`, `Movies`, …).
    ///
    /// # Returns
    /// A [`ProfileUgcsResult`] whose `nodes` vector contains every UGC node
    /// collected across all pages.  The `page_info` field reflects the state
    /// of the **last** page fetched (i.e. `has_next_page` will always be
    /// `false` on a successful return).
    #[tracing::instrument(name = "ugcs.get_users_ugcs", skip(self),
        fields(profile_id = %profile_id, ugc_type = ?ugc_type))]
    pub async fn get_users_ugcs(
        &self,
        profile_id: &str,
        ugc_type:   UgcType,
    ) -> Result<ProfileUgcsResult> {
        let session    = self.session.get().await?;
        let bearer     = session.bearer();
        let type_str   = ugc_type.as_str();
        let game_id    = "j68d";

        let mut all_nodes: Vec<ProfileUgcNode> = Vec::new();
        let mut cursor:    Option<String>       = None;

        loop {
            // ── Build the page input ──────────────────────────────────────
            let page_input = match &cursor {
                None    => serde_json::json!({ "first": 50 }),
                Some(c) => serde_json::json!({ "after": c, "first": 50 }),
            };

            let payload = serde_json::json!({
                "id": "GetProfileUgcs-E93B01BCA08092B6B85CB1734A466E36",
                "variables": {
                    "gameId":    game_id,
                    "profileId": profile_id,
                    "type":      type_str,
                    "pageInput": page_input,
                },
            });

            // ── Send request ──────────────────────────────────────────────
            let response = self
                .http
                .post(FEDERATION_GRAPHQL_ENDPOINT)
                .headers(self.headers(&bearer))
                .json(&payload)
                .send()
                .await
                .map_err(|e| MspError::from_wreq(e, EP))?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(MspError::api(EP, status.as_u16(), body));
            }

            let bytes = response.bytes().await
                .map_err(|e| MspError::from_wreq(e, EP))?;
            let body: Value = serde_json::from_slice(&bytes)
                .map_err(|e| MspError::deserialize(e, EP))?;

            if let Some(errors) = body.get("errors") {
                return Err(MspError::graphql(EP, errors.to_string()));
            }

            // ── Navigate the response tree ────────────────────────────────
            let ugcs_value = &body["data"]["profiles"]["byId"]["ugcs"];

            if ugcs_value.is_null() {
                return Ok(ProfileUgcsResult {
                    page_info: crate::models::ProfileUgcPageInfo {
                        next_page:     None,
                        has_next_page: false,
                    },
                    nodes: Vec::new(),
                });
            }

            let page: ProfileUgcsResult = serde_json::from_value(ugcs_value.clone())
                .map_err(|e| MspError::deserialize(e, EP))?;

            all_nodes.extend(page.nodes);

            if page.page_info.has_next_page {
                cursor = page.page_info.next_page.clone();

                if cursor.is_none() {
                    tracing::warn!(
                        "get_users_ugcs: has_next_page=true but no next_page cursor; stopping"
                    );
                    return Ok(ProfileUgcsResult {
                        page_info: page.page_info,
                        nodes:     all_nodes,
                    });
                }
            } else {
                return Ok(ProfileUgcsResult {
                    page_info: page.page_info,
                    nodes:     all_nodes,
                });
            }
        }
    }
}

// ── Helpers privés ────────────────────────────────────────────────────────────

/// Reconstruit le body BSON de la même façon que `We()` côté client web.
fn build_pgc_body(
    data:           &[u8],
    title:          &str,
    ugc_type:       &str,
    privacy_status: &str,
) -> std::result::Result<Vec<u8>, bson::ser::Error> {
    use bson::{Bson, Document};

    let mut resource = Document::new();
    resource.insert("data",         Bson::Binary(bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes:   data.to_vec(),
    }));
    resource.insert("extension",    Bson::String(String::new()));
    resource.insert("resourceType", Bson::String("PgcV1".to_owned()));

    let mut root = Document::new();
    root.insert("Resources",           Bson::Array(vec![Bson::Document(resource)]));
    root.insert("DefaultSnapshotType", Bson::Null);
    root.insert("ParticipantIds",      Bson::Null);
    root.insert("PrivacyStatus",       Bson::String(privacy_status.to_owned()));
    root.insert("Title",               Bson::String(title.to_owned()));
    root.insert("Type",                Bson::String(ugc_type.to_owned()));

    let mut out = Vec::new();
    root.to_writer(&mut out)?;
    Ok(out)
}

/// Equivalent de `qe()` côté client web : HMAC-SHA256 préfixé de `"5"`.
fn compute_signature(data: &[u8]) -> std::result::Result<String, String> {
    type HmacSha256 = Hmac<Sha256>;

    // Bytes UTF-8 bruts de la chaîne littérale, pas un décodage base64.
    let key_bytes = HMAC_KEY_STR.as_bytes();

    let mut mac = HmacSha256::new_from_slice(key_bytes)
        .map_err(|e| format!("HMAC init: {e}"))?;

    mac.update(data);

    let result = mac.finalize().into_bytes();
    let b64    = BASE64.encode(result);

    Ok(format!("5{b64}"))
}