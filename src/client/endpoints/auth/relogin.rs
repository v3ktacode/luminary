// src/client/endpoints/auth/relogin.rs
//
// The background re-login task.
//
// After a successful login(), this task runs in the background for the entire
// lifetime of the MspClient.  Every 2–3 hours (randomised) it runs the full
// login flow again — password grant, profile lookup, refresh grant — and
// replaces the stored session with a fresh one.  If presence is enabled, it
// also restarts the presence WebSocket so the new tokens are used there too.
//
// The task exits cleanly when the shutdown watch channel fires, which happens
// when MspClient is dropped or shutdown() is called.

use std::sync::Arc;
use std::time::Duration;

use rand::Rng;

use super::{
    grants, presence, timeouts, AuthEndpoint, PresenceSlot, Secret,
};

/// Spawns the background re-login task.
///
/// The task runs until the `shutdown` receiver sees a `true` value (sent by
/// `MspClient::shutdown()` or by the `Drop` impl).
///
/// `presence_enabled` is snapshotted from the `AtomicBool` at login time —
/// see the note in `mod.rs` about why changes after login aren't reflected.
pub(super) fn spawn_relogin(
    endpoint:         &AuthEndpoint<'_>,
    username:         String,
    password:         Secret,
    region:           String,
    slot:             PresenceSlot,
    presence_enabled: bool,
    mut shutdown:     tokio::sync::watch::Receiver<bool>,
) {
    use timeouts::{RELOGIN_MAX_SECS, RELOGIN_MIN_SECS};

    let http            = endpoint.http.clone();
    let session         = endpoint.session.clone();
    let event_bus       = endpoint.event_bus.clone();
    let device_id       = endpoint.device_id.to_owned();
    let config          = Arc::clone(&endpoint.config);
    let presence_atomic = Arc::clone(endpoint.presence);

    tokio::spawn(async move {
        loop {
            let delay_secs = rand::thread_rng().gen_range(RELOGIN_MIN_SECS..=RELOGIN_MAX_SECS);
            tracing::debug!(delay_secs, "Next automatic re-login scheduled.");

            // Wait for either the shutdown signal or the re-login timer,
            // whichever comes first.
            tokio::select! {
                biased;
                _ = shutdown.changed() => {
                    tracing::info!("Re-login task received shutdown signal.");
                    break;
                }
                _ = tokio::time::sleep(Duration::from_secs(delay_secs)) => {}
            }

            // The select above may have exited because the channel was
            // closed rather than because a true value was sent. Check
            // explicitly before doing any work.
            if *shutdown.borrow() {
                break;
            }

            tracing::info!("Executing scheduled re-login for '{username}'…");

            // We need an AuthEndpoint to reuse the grant functions, but this
            // background task owns its own resources rather than borrowing
            // from the original endpoint.  We create a dummy shutdown sender
            // that is never actually signalled — the real shutdown is handled
            // by the select above.  This is a known limitation of the current
            // design; a future refactor could extract the grant logic into
            // free functions that don't need an AuthEndpoint at all.
            let dummy_shutdown  = Arc::new(tokio::sync::watch::channel(false).0);
            let presence_atomic_ref = Arc::clone(&presence_atomic);
            let endpoint = AuthEndpoint {
                http:      &http,
                session:   &session,
                device_id: &device_id,
                event_bus: &event_bus,
                config:    Arc::clone(&config),
                shutdown:  &dummy_shutdown,
                presence:  &presence_atomic_ref,
            };

            match grants::run_login_flow(&endpoint, &username, &password.0, &region).await {
                Ok(new_session) => {
                    session.set(new_session.clone()).await;
                    tracing::info!(
                        profile_id = %new_session.profile_id,
                        "Re-login successful — session refreshed."
                    );
                    if presence_enabled {
                        presence::spawn_presence(
                            &endpoint,
                            new_session,
                            region.clone(),
                            slot.clone(),
                        )
                        .await;
                    } else {
                        tracing::debug!(
                            "Presence disabled — skipping presence spawn after re-login."
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "Scheduled re-login failed: {e:?}. Will retry next cycle."
                    );
                }
            }
        }

        tracing::debug!("Re-login task exiting.");
    });
}