// src/client/endpoints/auth/mod.rs
//
// Everything authentication-related lives here, split across four files:
//
//   mod.rs          — AuthEndpoint struct, login() and refresh() public API.
//   grants.rs       — The OAuth grant methods (password grant, refresh grant,
//                     profile id resolution).
//   presence.rs     — The Presence WebSocket supervisor and all the frame
//                     handling that goes with it.
//   relogin.rs      — The background task that re-runs the full login flow
//                     every 2–3 hours to keep the session alive.
//
// ## How a login actually works
//
// MSP2 uses a two-step OAuth2 flow:
//
//   1. Password grant  →  we get a generic access + refresh token pair.
//   2. Refresh grant   →  we exchange that for a *profile-scoped* session
//      (the `profileId` is baked into `acr_values` so the resulting token
//      is tied to one specific in-game character).
//
// Between those two steps we call the Profile Identity API to turn the
// opaque `sub` claim from the JWT into an actual `profileId`.
//
// ## What keeps running after login()
//
//   • A Presence WebSocket supervisor (if presence is enabled).
//     It maintains the real-time connection and reconnects automatically
//     on any transient failure.
//
//   • A re-login task that fires every 2–3 hours (randomised) to rotate
//     the full session before the tokens expire.  It replaces both tokens
//     *and* restarts the presence connection.

mod grants;
mod presence;
mod relogin;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Mutex;
use tokio::task::AbortHandle;
use wreq::Client;

use crate::{
    config::MspConfig,
    errors::Result,
    event_bus::EventBus,
    models::MspSession,
    session::SessionStore,
};

// ---------------------------------------------------------------------------
// Shared constants
// ---------------------------------------------------------------------------

/// The `Origin` header sent with every request.  The server validates this.
pub(super) const ORIGIN: &str = "https://moviestarplanet2.com";

/// The game identifier used across OAuth scopes, presence handshakes, and
/// various API path segments.
pub(super) const GAME_ID: &str = "j68d";

/// OAuth client credentials.  These can be overridden via environment
/// variables (`MSP_CLIENT_ID` / `MSP_CLIENT_SECRET`) but the defaults are
/// the ones the official Unity client uses.
pub(super) const CLIENT_ID: &str = "unity.client";
pub(super) const CLIENT_SECRET: &str = "secret";

/// A short label attached to every `MspError` produced by this module so
/// callers can tell at a glance where an error originated.
pub(super) const EP: &str = "auth";

// ---------------------------------------------------------------------------
// Shared timing constants
// ---------------------------------------------------------------------------

pub(super) mod timeouts {
    use std::time::Duration;

    // Re-login window: pick a random moment between 2 h and 3 h after the
    // previous login so that multiple clients running in parallel don't all
    // hammer the token endpoint at exactly the same time.
    pub const RELOGIN_MIN_SECS: u64 = 2 * 60 * 60;
    pub const RELOGIN_MAX_SECS: u64 = 3 * 60 * 60;

    // Login retry policy.  Currently set to 1 attempt (no retries) — the
    // loop infrastructure is already in place if we ever want to raise this.
    pub const LOGIN_MAX_ATTEMPTS: u32 = 1;
    pub const LOGIN_BACKOFF_BASE: Duration = Duration::from_secs(2);
    pub const LOGIN_BACKOFF_MAX: Duration = Duration::from_secs(30);

    // Presence WebSocket tuning.
    //
    // BACKOFF_MIN / BACKOFF_MAX control how long we wait between reconnect
    // attempts when the socket drops unexpectedly.  The delay doubles on
    // every failure up to the max, with an extra random jitter on top.
    pub const PRESENCE_BACKOFF_MIN: Duration = Duration::from_secs(2);
    pub const PRESENCE_BACKOFF_MAX: Duration = Duration::from_secs(60);

    // How often we send our own application-level heartbeat (message type
    // "500") to tell the presence server we're still alive.
    pub const PRESENCE_PING_INTERVAL: Duration = Duration::from_secs(5);

    // How often we send a raw Engine.IO ping ("2") independently of the
    // application heartbeat above.
    pub const PRESENCE_ENGINE_PING_INTERVAL: Duration = Duration::from_secs(10);

    // If we receive nothing from the server for this long, we consider the
    // connection dead and trigger a reconnect.
    pub const PRESENCE_READ_TIMEOUT: Duration = Duration::from_secs(45);

    // Maximum time we'll wait for each individual handshake frame during
    // the WebSocket connection setup.
    pub const PRESENCE_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
}

// ---------------------------------------------------------------------------
// Shared internal type aliases
// ---------------------------------------------------------------------------

/// Holds the abort handle for the currently-running presence task.
/// Wrapped in `Arc<Mutex<Option<…>>>` so it can be shared between the
/// initial login call and the re-login background task.
pub(super) type PresenceSlot = Arc<Mutex<Option<AbortHandle>>>;

/// The error type used inside the WebSocket machinery.  We use a boxed trait
/// object because the socket stream and sink can each produce their own
/// concrete error types, and we don't want to tie this code to either.
pub(super) type WsError = Box<dyn std::error::Error + Send + Sync>;

// ---------------------------------------------------------------------------
// Password memory hygiene
// ---------------------------------------------------------------------------

/// A thin wrapper around a password string that zeroes the memory when
/// dropped.
///
/// This limits how long the plaintext password lives in process memory.
/// Note that Rust's optimiser is technically allowed to elide writes to
/// memory it considers "dead" — a proper `zeroize`-based solution would be
/// more robust, but this covers the most common case.
pub(super) struct Secret(pub(super) String);

impl Drop for Secret {
    fn drop(&mut self) {
        // SAFETY: we own the String exclusively and are about to drop it;
        // overwriting the bytes before the deallocation is intentional.
        unsafe { self.0.as_bytes_mut().iter_mut().for_each(|b| *b = 0) };
    }
}

// ---------------------------------------------------------------------------
// OAuth client credentials
// ---------------------------------------------------------------------------

/// Resolves the OAuth client credentials at runtime, preferring environment
/// variables over the compiled-in defaults.  Results are cached in a
/// `OnceLock` so the env is only read once per process.
pub(super) mod credentials {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use std::sync::OnceLock;

    use super::{CLIENT_ID, CLIENT_SECRET};

    pub fn client_id() -> &'static str {
        static ID: OnceLock<String> = OnceLock::new();
        ID.get_or_init(|| {
            std::env::var("MSP_CLIENT_ID").unwrap_or_else(|_| CLIENT_ID.to_owned())
        })
    }

    pub fn client_secret() -> &'static str {
        static SECRET: OnceLock<String> = OnceLock::new();
        SECRET.get_or_init(|| {
            std::env::var("MSP_CLIENT_SECRET").unwrap_or_else(|_| CLIENT_SECRET.to_owned())
        })
    }

    /// Produces the `Basic <base64(id:secret)>` header value expected by the
    /// token endpoint.
    pub fn basic_auth() -> String {
        format!(
            "Basic {}",
            STANDARD.encode(format!("{}:{}", client_id(), client_secret()))
        )
    }
}

// ---------------------------------------------------------------------------
// AuthEndpoint
// ---------------------------------------------------------------------------

/// Handles all authentication operations for a single `MspClient`.
///
/// You get one of these by calling `client.auth()` — it borrows the client
/// for the duration of the call and doesn't own anything itself.
///
/// ```rust,ignore
/// let session = client.auth().login("MyUsername", "hunter2", "FR").await?;
/// ```
pub struct AuthEndpoint<'c> {
    pub(crate) http:      &'c Client,
    pub(crate) session:   &'c SessionStore,
    pub(crate) device_id: &'c str,
    pub(crate) event_bus: &'c EventBus,
    pub(crate) config:    Arc<MspConfig>,
    pub(crate) shutdown:  &'c Arc<tokio::sync::watch::Sender<bool>>,
    pub(crate) presence:  &'c Arc<AtomicBool>,
}

impl<'c> AuthEndpoint<'c> {
    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Authenticates with MovieStarPlanet 2 and starts the background tasks
    /// that keep the session alive.
    ///
    /// Runs the full two-step OAuth flow (password grant → profile resolution
    /// → refresh grant), stores the resulting session, then optionally starts
    /// the Presence WebSocket and the automatic re-login timer.
    ///
    /// # Arguments
    ///
    /// * `username` — the account username.
    /// * `password` — the account password.
    /// * `region`   — the two-letter region code, e.g. `"FR"` or `"US"`.
    ///   Casing doesn't matter — it's normalised to uppercase internally.
    ///
    /// # Errors
    ///
    /// Returns `MspError::InvalidCredentials` for wrong username/password,
    /// `MspError::AccountBanned` for permanently banned accounts, or
    /// `MspError::AuthFailed` for other login rejections.
    #[tracing::instrument(
        name = "msp_login",
        skip_all,
        fields(username = %username, region = %region, device_id = %self.device_id)
    )]
    pub async fn login(
        &self,
        username: &str,
        password: &str,
        region: &str,
    ) -> Result<MspSession> {
        let region = region.to_uppercase();
        let session = grants::run_login_flow(self, username, password, &region).await?;

        self.session.set(session.clone()).await;
        tracing::info!(profile_id = %session.profile_id, "Session successfully initialized.");

        // Snapshot the presence flag now.  The re-login task will use this
        // value for the lifetime of the session — if you change the flag
        // after calling login(), the running tasks won't see the update.
        let presence_enabled = self.presence.load(Ordering::Relaxed);

        // The PresenceSlot is shared between the initial spawn here and the
        // re-login task below, so the re-login task can abort the old
        // connection before starting a fresh one.
        let slot: PresenceSlot = Arc::new(Mutex::new(None));

        if presence_enabled {
            presence::spawn_presence(self, session.clone(), region.clone(), slot.clone()).await;
        } else {
            tracing::info!("Presence WebSocket skipped (presence flag is false).");
        }

        relogin::spawn_relogin(
            self,
            username.to_owned(),
            Secret(password.to_owned()),
            region,
            slot,
            presence_enabled,
            self.shutdown.subscribe(),
        );

        Ok(session)
    }

    /// Silently rotates the access token using the stored refresh token.
    ///
    /// This is called automatically by other endpoints when they detect an
    /// expired token, but you can also call it manually if you want to
    /// pre-emptively refresh before making a batch of requests.
    ///
    /// The session store is updated in place — callers that held a reference
    /// to the old session should re-fetch it afterwards.
    ///
    /// # Errors
    ///
    /// Returns `MspError::NoSession` if `login()` hasn't been called yet.
    /// Any error from the token endpoint is propagated as-is.
    #[tracing::instrument(name = "msp_refresh", skip_all)]
    pub async fn refresh(&self) -> Result<()> {
        let session = self.session.get().await?;
        tracing::debug!(
            profile_id = %session.profile_id,
            "Rotating access token silently…"
        );

        let new_session = grants::refresh_grant(
            self,
            &session.refresh_token,
            &grants::acr_base(self),
            &session.profile_id,
            &session.sub_id,
            &session.region,
        )
        .await?;

        self.session.set(new_session).await;
        tracing::info!("Access token rotated successfully.");
        Ok(())
    }
}