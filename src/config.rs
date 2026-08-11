//! Global configuration for the MSP client.

use std::time::Duration;

/// Central configuration record.
#[derive(Debug, Clone)]
pub struct MspConfig {
    // ── Identity ──────────────────────────────────────────────────────────
    pub game_id:        String,
    pub client_id:      String,
    pub client_secret:  String,

    // ── Base URLs ─────────────────────────────────────────────────────────
    pub base_url_eu:        String,
    pub base_url_eu_secure: String,
    pub gameserver_eu_ws:   String,
    pub origin:             String,
    pub referer:            String,

    // ── HTTP tuning ───────────────────────────────────────────────────────
    pub request_timeout_ms: u64,
    pub connect_timeout_ms: u64,
}

impl Default for MspConfig {
    fn default() -> Self {
        Self {
            game_id:       env_or("MSP_GAME_ID",       "j68d"),
            client_id:     env_or("MSP_CLIENT_ID",     "unity.client"),
            client_secret: env_or("MSP_CLIENT_SECRET", "secret"),

            base_url_eu:        env_or("MSP_BASE_URL_EU",        "https://eu.mspapis.com"),
            base_url_eu_secure: env_or("MSP_BASE_URL_EU_SECURE", "https://eu-secure.mspapis.com"),
            gameserver_eu_ws:   env_or("MSP_GAMESERVER_EU_WS",   "wss://gameserver-eu.mspapis.com"),
            origin:             env_or("MSP_ORIGIN",             "https://moviestarplanet2.com"),
            referer:            env_or("MSP_REFERER",            "https://moviestarplanet2.com/"),

            request_timeout_ms: env_u64("MSP_REQUEST_TIMEOUT_MS", 30_000),
            connect_timeout_ms: env_u64("MSP_CONNECT_TIMEOUT_MS", 10_000),
        }
    }
}

impl MspConfig {
    pub fn builder() -> MspConfigBuilder {
        MspConfigBuilder(Self::default())
    }

    // ── Dynamic Regional Domain Resolvers ──────────────────────────────────
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url_eu
    }

    #[must_use]
    pub fn get_base_url_regional(&self, region: &str) -> String {
        let r = region.to_uppercase();
        if r == "US" || r == "CA" {
            "https://us.mspapis.com".to_string()
        } else {
            self.base_url_eu.clone()
        }
    }

    #[must_use]
    pub fn get_secure_url_regional(&self, region: &str) -> String {
        let r = region.to_uppercase();
        if r == "US" || r == "CA" {
            "https://us-secure.mspapis.com".to_string()
        } else {
            self.base_url_eu_secure.clone()
        }
    }

    #[must_use]
    pub fn get_gameserver_ws_regional(&self, region: &str) -> String {
        let r = region.to_uppercase();
        if r == "US" || r == "CA" {
            "wss://gameserver-us.mspapis.com".to_string()
        } else {
            self.gameserver_eu_ws.clone()
        }
    }

    // ── Derived URL helpers ───────────────────────────────────────────────

    #[must_use]
    pub fn token_endpoint(&self) -> String {
        format!("{}/loginidentity/connect/token", self.base_url_eu_secure)
    }

    #[must_use]
    pub fn token_endpoint_regional(&self, region: &str) -> String {
        format!("{}/loginidentity/connect/token", self.get_secure_url_regional(region))
    }

    #[must_use]
    pub fn profiles_for_sub(&self, sub: &str) -> String {
        format!("{}/profileidentity/v1/logins/{sub}/profiles", self.base_url_eu)
    }

    #[must_use]
    pub fn profiles_for_sub_regional(&self, sub: &str, region: &str) -> String {
        format!("{}/profileidentity/v1/logins/{sub}/profiles", self.get_base_url_regional(region))
    }
    
    /// Comments GraphQL gateway for the given region.
    ///
    /// Mirrors `get_base_url_regional`: EU accounts hit `eu.mspapis.com`,
    /// US/CA accounts hit `us.mspapis.com`.
    #[must_use]
    pub fn comments_endpoint_regional(&self, region: &str) -> String {
        format!("{}/edgecomments/graphql", self.get_base_url_regional(region))
    }

    /// Experience (XP/level) endpoint for a given profile, routed to the
    /// cluster matching the account's region.
    #[must_use]
    pub fn experience_regional(&self, profile_id: &str, region: &str) -> String {
        format!(
            "{}/experience/v1/profiles/{profile_id}/games/{}/experience",
            self.get_base_url_regional(region), self.game_id,
        )
    }

    /// GraphQL gateway for relationships (friends, friend requests, blocks),
    /// routed to the cluster matching the account's region.
    #[must_use]
    pub fn relationships_graphql_regional(&self, region: &str) -> String {
        format!("{}/edgerelationships/graphql", self.get_base_url_regional(region))
    }

    /// REST endpoint for accepting/rejecting a specific friend request.
    ///
    /// `requester_profile_id` is the profile that sent the request;
    /// `responder_profile_id` is the profile responding to it (normally the
    /// logged-in user).
    #[must_use]
    pub fn relationship_request_regional(
        &self,
        requester_profile_id: &str,
        responder_profile_id: &str,
        region: &str,
    ) -> String {
        format!(
            "{}/profilerelationships/v2/profiles/{requester_profile_id}/relationships/requests/{responder_profile_id}",
            self.get_base_url_regional(region),
        )
    }

    /// Federation-gateway GraphQL endpoint for greetings, routed to the
    /// cluster matching the account's region.
    #[must_use]
    pub fn greetings_endpoint_regional(&self, region: &str) -> String {
        format!("{}/federationgateway/graphql", self.get_base_url_regional(region))
    }

    #[must_use]
    pub fn profile_identity(&self, profile_id: &str) -> String {
        format!("{}/profileidentity/v1/profiles/{profile_id}", self.base_url_eu)
    }

    #[must_use]
    pub fn presence_ws(&self) -> String {
        format!(
            "{}/presenceserver/instance/socket.io?EIO=3&transport=websocket",
            self.gameserver_eu_ws,
        )
    }

    #[must_use]
    pub fn presence_ws_regional(&self, region: &str) -> String {
        format!(
            "{}/presenceserver/instance/socket.io?EIO=3&transport=websocket",
            self.get_gameserver_ws_regional(region),
        )
    }

    #[must_use]
    pub fn reservations(&self) -> String {
        format!("{}/matchmaker/v1/games/{}/reservations/", self.base_url_eu, self.game_id)
    }

    #[must_use]
    pub fn reservations_regional(&self, region: &str) -> String {
        format!(
            "{}/matchmaker/v1/games/{}/reservations/",
            self.get_base_url_regional(region),
            self.game_id
        )
    }

    #[must_use]
    pub fn attributes(&self, profile_id: &str) -> String {
        format!(
            "{}/profileattributes/v1/profiles/{profile_id}/games/{}/attributes",
            self.base_url_eu, self.game_id,
        )
    }

    #[must_use]
    pub fn collects(&self, profile_id: &str) -> String {
        format!(
            "{}/profilecollects/v3/profiles/{profile_id}/games/{}/collects",
            self.base_url_eu, self.game_id,
        )
    }

    #[must_use]
    pub fn collects_claim(&self, profile_id: &str) -> String {
        format!("{}/claim", self.collects(profile_id))
    }

    #[must_use]
    pub fn time_limited_reward(&self, profile_id: &str, reward_type: &str) -> String {
        format!(
            "{}/timelimitedrewards/v2/profiles/{profile_id}/games/{}/rewards/{reward_type}",
            self.base_url_eu, self.game_id,
        )
    }

    #[must_use]
    pub fn quests(&self, profile_id: &str) -> String {
        format!(
            "{}/quests/v2/profiles/{profile_id}/games/{}/quests",
            self.base_url_eu, self.game_id,
        )
    }

    #[must_use]
    pub fn conversations_by_profile(&self, profile_id: &str, other: &str) -> String {
        format!(
            "{}/gamemessaging/v1/profiles/{profile_id}/conversations/profiles/{other}",
            self.base_url_eu,
        )
    }

    #[must_use]
    pub fn conversations_create(&self, creator_id: &str) -> String {
        format!(
            "{}/gamemessaging/v1/conversations?creator={creator_id}",
            self.base_url_eu,
        )
    }

    #[must_use]
    pub fn conversation_history(&self, conversation_id: &str) -> String {
        format!(
            "{}/gamemessaging/v1/conversations/{conversation_id}/history",
            self.base_url_eu,
        )
    }

    #[must_use]
    pub fn conversation_participant(&self, conversation_id: &str, profile_id: &str) -> String {
        format!(
            "{}/gamemessaging/v1/conversations/{conversation_id}/participants/{profile_id}",
            self.base_url_eu,
        )
    }

    #[must_use]
    pub fn conversations_list(&self, profile_id: &str) -> String {
        format!(
            "{}/gamemessaging/v1/participants/{profile_id}/conversations",
            self.base_url_eu,
        )
    }

    // ── Timeout helpers ───────────────────────────────────────────────────

    #[must_use]
    pub fn request_timeout(&self) -> Duration {
        Duration::from_millis(self.request_timeout_ms)
    }

    #[must_use]
    pub fn connect_timeout(&self) -> Duration {
        Duration::from_millis(self.connect_timeout_ms)
    }

    // ── basic_auth helper ────────────────────────────────────────────────

    #[must_use]
    pub fn basic_auth(&self) -> String {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let encoded = STANDARD.encode(format!("{}:{}", self.client_id, self.client_secret));
        format!("Basic {encoded}")
    }
}

// ── Builder ───────────────────────────────────────────────────────────────────

pub struct MspConfigBuilder(MspConfig);

impl MspConfigBuilder {
    pub fn game_id(mut self, v: impl Into<String>)        -> Self { self.0.game_id = v.into();        self }
    pub fn client_id(mut self, v: impl Into<String>)      -> Self { self.0.client_id = v.into();      self }
    pub fn client_secret(mut self, v: impl Into<String>)  -> Self { self.0.client_secret = v.into();  self }
    pub fn base_url_eu(mut self, v: impl Into<String>)    -> Self { self.0.base_url_eu = v.into();    self }
    pub fn base_url_eu_secure(mut self, v: impl Into<String>) -> Self {
        self.0.base_url_eu_secure = v.into(); self
    }
    pub fn gameserver_eu_ws(mut self, v: impl Into<String>) -> Self {
        self.0.gameserver_eu_ws = v.into(); self
    }
    pub fn origin(mut self, v: impl Into<String>)         -> Self { self.0.origin = v.into();         self }
    pub fn referer(mut self, v: impl Into<String>)        -> Self { self.0.referer = v.into();        self }
    pub fn request_timeout_ms(mut self, v: u64)           -> Self { self.0.request_timeout_ms = v;    self }
    pub fn connect_timeout_ms(mut self, v: u64)           -> Self { self.0.connect_timeout_ms = v;    self }

    pub fn build(self) -> MspConfig { self.0 }
}

fn env_or(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.to_owned())
}

fn env_u64(var: &str, default: u64) -> u64 {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}