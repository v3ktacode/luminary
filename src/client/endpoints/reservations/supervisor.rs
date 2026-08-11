// src/client/endpoints/reservations/supervisor.rs
//
// The outer retry loop for a quiz session. `run_quiz_session` (in
// `session.rs`) handles a single WebSocket connection from handshake to
// disconnect; this module is what keeps calling it again after every drop,
// with exponential back-off, until either the consumer stops listening or
// the daily reward limit is detected.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use tokio::sync::{mpsc, RwLock};
use wreq::Client;

use super::quiz_config::{QuizConfig, QuizStats};
use super::session::run_quiz_session;
use crate::events::QuizEvent;

/// How many consecutive zero-reward rounds must be observed before the
/// supervisor concludes the daily limit has been reached and exits.
pub(super) const DAILY_LIMIT_ZERO_ROUNDS_THRESHOLD: u32 = 1;

/// What the inner session loop wants the supervisor to do next.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SessionOutcome {
    /// Normal disconnect — reconnect after back-off.
    Reconnect,
    /// The receiver channel was dropped — stop silently.
    ReceiverGone,
    /// Both XP and SC were 0 for enough consecutive `RoundEnd`s to hit
    /// `DAILY_LIMIT_ZERO_ROUNDS_THRESHOLD`. The supervisor exits and the
    /// process terminates — see the note on `play_star_quiz_ex`.
    DailyLimitReached,
}

pub(super) async fn quiz_supervisor(
    http: &Client,
    socket_url: &str,
    access_token: &str,
    answer_key: &Arc<RwLock<HashMap<String, u32>>>,
    localization: &HashMap<String, String>,
    config: &QuizConfig,
    tx: &mpsc::UnboundedSender<QuizEvent>,
    origin: &str,
    region: &str,
) {
    let stats = QuizStats::default();
    let connected_once = AtomicBool::new(false);
    let mut backoff = config.initial_backoff();
    // Tracks how many *consecutive* full rounds produced zero XP **and** zero SC.
    let mut consecutive_zero_rounds = 0u32;

    loop {
        if tx.is_closed() {
            tracing::info!("Receiver dropped — exiting supervisor.");
            break;
        }

        connected_once.store(false, Ordering::Relaxed);

        let outcome = match run_quiz_session(
            http, socket_url, access_token,
            answer_key, localization, config, tx, &stats,
            &connected_once, origin, region,
            &mut consecutive_zero_rounds,
        )
        .await
        {
            Ok(o) => {
                match o {
                    SessionOutcome::Reconnect => {
                        stats.sessions_completed.fetch_add(1, Ordering::Relaxed);
                        stats.total_reconnects.fetch_add(1, Ordering::Relaxed);
                    }
                    SessionOutcome::ReceiverGone => {
                        stats.sessions_completed.fetch_add(1, Ordering::Relaxed);
                    }
                    SessionOutcome::DailyLimitReached => {
                        // Accounted for below — just surface the outcome.
                    }
                }
                o
            }
            Err(e) => {
                stats.sessions_error.fetch_add(1, Ordering::Relaxed);
                stats.total_reconnects.fetch_add(1, Ordering::Relaxed);
                tracing::error!(error = %e, "Quiz socket error; reconnecting.");
                SessionOutcome::Reconnect
            }
        };

        match outcome {
            // ── Hard stops ────────────────────────────────────────────────
            SessionOutcome::ReceiverGone => {
                tracing::info!("Channel receiver gone — supervisor exiting.");
                break;
            }
            SessionOutcome::DailyLimitReached => {
                tracing::warn!(
                    consecutive_zero_rounds,
                    "Daily reward limit detected (XP=0 and SC=0 for {} consecutive \
                     round(s)). Shutting down.",
                    DAILY_LIMIT_ZERO_ROUNDS_THRESHOLD,
                );
                // Notify the consumer so it can display a final message
                // before we hard-exit. Ignore the send error — the channel
                // might already be closing.
                let _ = tx.send(QuizEvent::DailyLimitReached);
                // Give the consumer a moment to process the event and
                // redraw before the process exits.
                tokio::time::sleep(Duration::from_millis(200)).await;
                std::process::exit(0);
            }
            // ── Reconnect ─────────────────────────────────────────────────
            SessionOutcome::Reconnect => {}
        }

        if connected_once.load(Ordering::Relaxed) {
            backoff = config.initial_backoff();
        }

        if !config.play_forever || tx.is_closed() {
            break;
        }

        let jitter = Duration::from_millis(rand::thread_rng().gen_range(0..=config.jitter_max_ms));
        let total = (backoff + jitter + config.reconnect_extra_delay()).min(config.max_backoff());
        tracing::info!(wait_ms = %total.as_millis(), "Waiting before reconnect…");
        tokio::time::sleep(total).await;
        backoff = (backoff * 2).min(config.max_backoff());
    }
}