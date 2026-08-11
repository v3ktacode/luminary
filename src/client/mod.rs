pub mod builder;
mod cookies;
pub mod http;
mod stealth;
pub mod endpoints;

pub use builder::{BrowserBrand, MspClientBuilder};
pub use endpoints::quests::{pending_random_daily_children, random_daily_children, DailyRemainingCounters};
pub use endpoints::highscores::TimeScope;
pub use stealth::StealthConfig;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wreq::Client;
use wreq_util::{Profile, Platform};
use tokio::sync::broadcast;

use crate::config::MspConfig;
use crate::event_bus::EventBus;
use crate::events::MspEvent;
use crate::session::SessionStore;
use crate::state::SessionState;
use cookies::PersistentJar;
use endpoints::{
    auth::AuthEndpoint,
    attributes::AttributesEndpoint,
    collects::CollectsEndpoint,
    comments::CommentsEndpoint,
    greetings::GreetingsEndpoint,
    highscores::HighscoresEndpoint,
    messaging::MessagingEndpoint,
    profiles::ProfilesEndpoint,
    reservations::ReservationsEndpoint,
    quests::QuestsEndpoint,
    ugcs::UgcsEndpoint,
    friends::FriendsEndpoint,
    pets::PetsEndpoint,
    experience::ExperienceEndpoint
};

pub struct MspClient {
    http:          Client,
    device_id:     String,
    session:       SessionStore,
    profile:       Profile,
    platform:      Platform,
    event_bus:     EventBus,
    proxy_url:     Option<String>,
    enforce_proxy: bool,
    jar:           Arc<PersistentJar>,
    stealth:       StealthConfig,
    config:        Arc<MspConfig>,
    shutdown:      Arc<tokio::sync::watch::Sender<bool>>,
    presence:      Arc<AtomicBool>,
}

impl MspClient {
    pub fn builder() -> MspClientBuilder { MspClientBuilder::default() }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        http:          Client,
        device_id:     String,
        profile:       Profile,
        platform:      Platform,
        proxy_url:     Option<String>,
        enforce_proxy: bool,
        jar:           Arc<PersistentJar>,
        stealth:       StealthConfig,
        config:        MspConfig,
    ) -> Self {
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);
        Self {
            http,
            device_id,
            session:   SessionStore::new(),
            profile,
            platform,
            event_bus: EventBus::new(),
            proxy_url,
            enforce_proxy,
            jar,
            stealth,
            config:    Arc::new(config),
            shutdown:  Arc::new(shutdown_tx),
            presence:  Arc::new(AtomicBool::new(true)),
        }
    }

    /// Configure whether the Presence WebSocket should be enabled.
    /// 
    /// Defaults to `true`. Set to `false` to disable presence entirely.
    pub fn set_presence(&self, enabled: bool) {
        self.presence.store(enabled, Ordering::Relaxed);
    }

    /// Returns whether presence is currently enabled.
    pub fn presence_enabled(&self) -> bool {
        self.presence.load(Ordering::Relaxed)
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown.send(true);
        tracing::debug!("Shutdown signal sent to all background tasks.");
    }

    pub fn events(&self) -> broadcast::Receiver<MspEvent> {
        self.event_bus.subscribe()
    }

    #[inline]
    pub async fn pace(&self) {
        self.stealth.pace().await;
    }

    #[inline]
    pub fn stealth(&self) -> &StealthConfig { &self.stealth }

    #[inline]
    pub fn config(&self) -> &MspConfig { &self.config }

    pub async fn export_state(&self) -> Option<SessionState> {
        let session     = self.session.get().await.ok()?;
        let exported_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        Some(SessionState {
            profile_id:              session.profile_id,
            sub_id:                  session.sub_id,
            device_id:               session.device_id,
            region:                  session.region,
            access_token:            session.access_token,
            refresh_token:           session.refresh_token,
            access_token_expires_at: session.access_token_expires_at,
            browser_profile:         format!("{:?}", self.profile),
            platform:                format!("{:?}", self.platform),
            proxy_url:               self.proxy_url.clone(),
            enforce_proxy:           self.enforce_proxy,
            cookies:                 self.jar.export(),
            exported_at,
            schema_version:          SessionState::SCHEMA_VERSION,
        })
    }

    #[inline] pub fn profile(&self)  -> Profile  { self.profile  }
    #[inline] pub fn platform(&self) -> Platform { self.platform }

    #[inline]
    pub fn auth(&self) -> AuthEndpoint<'_> {
        AuthEndpoint {
            http:      &self.http,
            session:   &self.session,
            device_id: &self.device_id,
            event_bus: &self.event_bus,
            config:    Arc::clone(&self.config),
            shutdown:  &self.shutdown,
            presence:  &self.presence,
        }
    }

    #[inline]
    pub fn attributes(&self) -> AttributesEndpoint<'_> {
        AttributesEndpoint {
            http:    &self.http,
            session: &self.session,
            config:  Arc::clone(&self.config),
        }
    }

    #[inline]
    pub fn collects(&self) -> CollectsEndpoint<'_> {
        CollectsEndpoint {
            http:    &self.http,
            session: &self.session,
            config:  Arc::clone(&self.config),
        }
    }

    #[inline]
    pub fn comments(&self) -> CommentsEndpoint<'_> {
        CommentsEndpoint {
            http:    &self.http,
            session: &self.session,
            config:  Arc::clone(&self.config),
        }
    }

    pub fn greetings(&self) -> GreetingsEndpoint<'_> {
        GreetingsEndpoint {
           http:    &self.http,
            session: &self.session,
            config:  Arc::clone(&self.config),
        }
    }

    #[inline]
    pub fn highscores(&self) -> HighscoresEndpoint<'_> {
        HighscoresEndpoint { http: &self.http, session: &self.session }
    }

    #[inline]
    pub fn messaging(&self) -> MessagingEndpoint<'_> {
        MessagingEndpoint {
            http:    &self.http,
            session: &self.session,
            config:  Arc::clone(&self.config),
        }
    }

    #[inline]
    pub fn profiles(&self) -> ProfilesEndpoint<'_> {
        ProfilesEndpoint { http: &self.http, session: &self.session }
    }

    #[inline]
    pub fn quests(&self) -> QuestsEndpoint<'_> {
        QuestsEndpoint {
            http:    &self.http,
            session: &self.session,
            config:  Arc::clone(&self.config),
        }
    }

    #[inline]
    pub fn reservations(&self) -> ReservationsEndpoint<'_> {
        ReservationsEndpoint {
            http:    &self.http,
            session: &self.session,
            config:  Arc::clone(&self.config),
        }
    }

    #[inline]
    pub fn ugcs(&self) -> UgcsEndpoint<'_> {
        UgcsEndpoint { http: &self.http, session: &self.session }
    }

    #[inline]
    pub fn friends(&self) -> FriendsEndpoint<'_> {
        FriendsEndpoint {
            http:    &self.http,
            session: &self.session,
            config:  Arc::clone(&self.config),
        }
    }

    pub fn pets(&self) -> PetsEndpoint<'_> {
        PetsEndpoint {
            http:    &self.http,
            session: &self.session,
            config:  Arc::clone(&self.config),
        }
    }

    #[inline]
    pub fn experience(&self) -> ExperienceEndpoint<'_> {
        ExperienceEndpoint {
            http:    &self.http,
            session: &self.session,
            config:  Arc::clone(&self.config),
        }
    }

    #[inline]
    pub fn session(&self) -> &SessionStore { &self.session }

    #[inline]
    pub fn raw_http(&self) -> &wreq::Client { &self.http }
}

impl Drop for MspClient {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        tracing::debug!("MspClient dropped — background tasks shutting down.");
    }
}