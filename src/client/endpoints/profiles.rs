use serde_json::Value;
use wreq::Client;

use crate::{
    errors::{MspError, Result},
    models::{ProfileAvatar, ProfileIdentity, ProfileNode, ProfileSearchResult, ProfileInfo},
    session::SessionStore,
};
use super::super::http::{build_headers, ContentType};

const EP: &'static str = "profiles";

const GRAPHQL_URL:        &str = "https://eu.mspapis.com/edgerelationships/graphql";
const GRAPHQL_PROFILE_URL: &str = "https://eu.mspapis.com/edgeprofile/graphql";
const IDENTITY_URL:       &str = "https://eu.mspapis.com/profileidentity/v1/profiles/{p}";
const GAME_ID:            &str = "j68d";

const SEARCH_QUERY: &str = "\
query GetProfileSearch(\
  $region: String!, $startsWith: String!, $pageSize: Int, \
  $currentPage: Int, $preferredGameId: String!\
) { findProfiles(region: $region, nameBeginsWith: $startsWith, pageSize: $pageSize, page: $currentPage) { \
    totalCount nodes { id avatar(preferredGameId: $preferredGameId) { gameId } } \
} }";

const GET_PROFILES_QUERY: &str = "\
query GetProfiles($profileIds: [String!]!, $gameId: String!) {\
  profiles(profileIds: $profileIds) {\
    id name culture avatar(preferredGameId: $gameId) { gameId } membership { lastTierExpiry } \
  } \
}";

const GET_PROFILE_QUERY: &str = r#"
            query GetProfile($profileId: String!, $gameId:String!) {
               profile(profileId: $profileId) {
                    name

                    balance(gameId: $gameId) {
                      available {
                        currency
                        count
                      }
                    }

                    memberships {
                      lastTierExpiry }}}
"#;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CurrencyBalance {
    pub currency: String,
    pub count:    i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProfileBalance {
    pub available: Vec<CurrencyBalance>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileMemberships {
    pub last_tier_expiry: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProfileDetail {
    pub name:        String,
    pub balance:     Option<ProfileBalance>,
    pub memberships: Option<ProfileMemberships>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub id:         String,
    pub name:       String,
    pub culture:    String,
    pub avatar:     Option<ProfileSummaryAvatar>,
    pub membership: Option<ProfileSummaryMembership>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummaryAvatar {
    pub game_id: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummaryMembership {
    pub last_tier_expiry: Option<String>,
}

pub struct ProfilesEndpoint<'c> {
    pub(crate) http:    &'c Client,
    pub(crate) session: &'c SessionStore,
}

impl<'c> ProfilesEndpoint<'c> {
    fn headers(&self, bearer: &str) -> wreq::header::HeaderMap {
        build_headers(
            ContentType::Json,
            Some(bearer),
            "https://moviestarplanet2.com",
            "https://moviestarplanet2.com/",
        )
    }

    async fn graphql(&self, payload: &serde_json::Value) -> Result<Value> {
        let session = self.session.get().await?;
        let response = self
            .http
            .post(GRAPHQL_URL)
            .headers(self.headers(&session.bearer()))
            .json(payload)
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
        serde_json::from_slice(&bytes)
            .map_err(|e| MspError::deserialize(e, EP))
    }

    async fn graphql_profile(&self, payload: &serde_json::Value) -> Result<Value> {
        let session = self.session.get().await?;
        let response = self
            .http
            .post(GRAPHQL_PROFILE_URL)
            .headers(self.headers(&session.bearer()))
            .json(payload)
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
        serde_json::from_slice(&bytes)
            .map_err(|e| MspError::deserialize(e, EP))
    }

    fn graphql_array<'v>(response: &'v Value, key: &str) -> Result<&'v Vec<Value>> {
        response["data"][key].as_array().ok_or_else(|| MspError::api(
            EP, 200,
            format!("Missing '{key}' array in GraphQL response"),
        ))
    }

    #[tracing::instrument(name = "profiles.get_profile", skip(self))]
    pub async fn get_profile(
        &self,
        profile_id: Option<&str>,
    ) -> Result<ProfileDetail> {
        let session   = self.session.get().await?;
        let target_id = profile_id.unwrap_or(&session.profile_id).to_owned();

        let variables_str = serde_json::json!({
            "profileId": target_id,
            "gameId":    GAME_ID,
        })
        .to_string();

        let payload = serde_json::json!({
            "query":     GET_PROFILE_QUERY,
            "variables": variables_str,
        });

        let response = self.graphql_profile(&payload).await?;

        if let Some(errors) = response.get("errors") {
            return Err(MspError::graphql(EP, errors.to_string()));
        }

        let profile_value = response
            .get("data")
            .and_then(|d| d.get("profile"))
            .ok_or_else(|| MspError::api(
                EP, 200,
                "Missing 'data.profile' in GetProfile response",
            ))?;

        serde_json::from_value(profile_value.clone())
            .map_err(|e| MspError::deserialize(e, EP))
    }

    #[tracing::instrument(name = "profiles.get_profiles", skip(self))]
    pub async fn get_profiles(&self, profile_ids: &[&str]) -> Result<Vec<ProfileInfo>> {
        let response = self
            .graphql(&serde_json::json!({
                "query":     GET_PROFILES_QUERY,
                "variables": { "profileIds": profile_ids, "gameId": GAME_ID },
            }))
            .await?;

        if let Some(errors) = response.get("errors") {
            return Err(MspError::graphql(EP, errors.to_string()));
        }

        Ok(Self::graphql_array(&response, "profiles")?
            .iter()
            .filter_map(|p| serde_json::from_value(p.clone()).ok())
            .collect())
    }

    #[tracing::instrument(name = "profiles.get_profile_summaries", skip(self))]
    pub async fn get_profile_summaries(
        &self,
        profile_ids: &[&str],
    ) -> Result<Vec<ProfileSummary>> {
        let response = self
            .graphql(&serde_json::json!({
                "query":     GET_PROFILES_QUERY,
                "variables": { "profileIds": profile_ids, "gameId": GAME_ID },
            }))
            .await?;

        if let Some(errors) = response.get("errors") {
            return Err(MspError::graphql(EP, errors.to_string()));
        }

        Ok(Self::graphql_array(&response, "profiles")?
            .iter()
            .filter_map(|p| serde_json::from_value(p.clone()).ok())
            .collect())
    }

    #[tracing::instrument(name = "profiles.search", skip(self))]
    pub async fn search_profiles(
        &self,
        username:       &str,
        region:         &str,
        page:           u32,
        page_size:      u32,
        game_id_filter: Option<&str>,
    ) -> Result<ProfileSearchResult> {
        let response = self
            .graphql(&serde_json::json!({
                "query": SEARCH_QUERY,
                "variables": {
                    "region":          region.to_uppercase(),
                    "startsWith":      username,
                    "pageSize":        page_size,
                    "currentPage":     page,
                    "preferredGameId": GAME_ID,
                },
            }))
            .await?;

        let raw         = &response["data"]["findProfiles"];
        let total_count = raw["totalCount"].as_u64().unwrap_or(0) as u32;

        let mut nodes: Vec<ProfileNode> = raw["nodes"]
            .as_array()
            .ok_or_else(|| MspError::api(
                EP, 200,
                "Missing 'nodes' in findProfiles",
            ))?
            .iter()
            .filter_map(|n| serde_json::from_value(n.clone()).ok())
            .collect();

        for node in &mut nodes {
            node.avatar
                .get_or_insert_with(|| ProfileAvatar { game_id: GAME_ID.into() });
        }

        if let Some(filter) = game_id_filter {
            nodes.retain(|n| {
                n.avatar.as_ref().map_or(false, |a| a.game_id == filter)
            });
        }

        Ok(ProfileSearchResult { total_count, nodes })
    }

    #[tracing::instrument(name = "profiles.get_identity", skip(self))]
    pub async fn get_profile_identity(
        &self,
        profile_id: &str,
    ) -> Result<Option<ProfileIdentity>> {
        let session = self.session.get().await?;
        let response = self
            .http
            .get(IDENTITY_URL.replace("{p}", profile_id))
            .headers(self.headers(&session.bearer()))
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

        if response.is_null() {
            return Ok(None);
        }

        if let Some(err) = response.get("error") {
            return Err(MspError::api(
                EP, 200,
                format!("Profile identity error: {}", err),
            ));
        }

        if response.is_object() {
            return serde_json::from_value(response)
                .map(Some)
                .map_err(|e| MspError::deserialize(e, EP));
        }

        let array = response.as_array().ok_or_else(|| MspError::api(
            EP, 200,
            format!("Unexpected response format: {:?}", response),
        ))?;

        array
            .first()
            .map(|v| serde_json::from_value(v.clone()).map_err(|e| MspError::deserialize(e, EP)))
            .transpose()
    }
}