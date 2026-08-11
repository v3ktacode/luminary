// src/client/endpoints/reservations/session.rs
//
// A single quiz WebSocket connection, from handshake through to disconnect:
// joining the room, answering questions (respecting `QuizConfig`), learning
// new answers from `Reveal` events, and detecting the daily-reward-limit
// heuristic. `supervisor.rs` is what re-runs this after it ends.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::task::JoinSet;
use wreq::ws::message::Message;
use wreq::Client;

use super::answer_key::pick_answer;
use super::localization::enrich;
use super::quiz_config::{QuizConfig, QuizStats};
use super::region::region_to_culture;
use super::supervisor::{SessionOutcome, DAILY_LIMIT_ZERO_ROUNDS_THRESHOLD};
use crate::events::{parse_quiz_frame, QuizEvent};

/// Outbound message channel buffer between the event loop and the dedicated
/// write task. 64 is comfortably more than we'd ever need to queue at once
/// (pings, answers, and the occasional chat message).
const WS_WRITE_BUFFER: usize = 64;

/// How long a given event signature is remembered for de-duplication.
/// The quiz server occasionally re-sends the same frame within a short
/// window; anything older than this is treated as a legitimately new event
/// even if it happens to hash the same.
const DEDUP_TTL: Duration = Duration::from_millis(800);

/// Once the dedup history grows past this many entries, it gets swept of
/// anything older than `DEDUP_TTL` rather than growing unbounded for the
/// lifetime of a long quiz session.
const DEDUP_PURGE_THRESHOLD: usize = 256;
const DEDUP_SHRINK_TARGET: usize = DEDUP_PURGE_THRESHOLD / 2;

// ---------------------------------------------------------------------------
// Event de-duplication
// ---------------------------------------------------------------------------

/// Tracks recently-seen event signatures so the same frame arriving twice in
/// a short window (which does happen) doesn't get forwarded to the consumer
/// or double-counted in the stats.
struct EventDeduplicator {
    history: HashMap<u64, Instant>,
    ttl: Duration,
}

impl EventDeduplicator {
    fn new(ttl: Duration) -> Self { Self { history: HashMap::new(), ttl } }

    fn is_duplicate(&mut self, sig: u64) -> bool {
        let now = Instant::now();
        if let Some(&ts) = self.history.get(&sig) {
            if now.duration_since(ts) < self.ttl {
                return true;
            }
        }
        self.history.insert(sig, now);

        if self.history.len() > DEDUP_PURGE_THRESHOLD {
            let mut retained = HashMap::with_capacity(DEDUP_SHRINK_TARGET);
            for (k, ts) in self.history.drain() {
                if now.duration_since(ts) < self.ttl {
                    retained.insert(k, ts);
                }
            }
            self.history = retained;
        }

        false
    }
}

/// Hashes the parts of an event that identify "the same occurrence" rather
/// than its full contents — e.g. two `Init` events are the same occurrence
/// if they carry the same question and answers, regardless of anything else
/// attached to them.
fn event_signature(event: &QuizEvent) -> u64 {
    let mut h = DefaultHasher::new();
    std::mem::discriminant(event).hash(&mut h);
    match event {
        QuizEvent::Init(ev) => { ev.question.hash(&mut h); ev.answers.hash(&mut h); }
        QuizEvent::QuestionShown(ev) => { ev.question.hash(&mut h); ev.answers.hash(&mut h); }
        QuizEvent::Reveal(ev) => { ev.correct_answer.hash(&mut h); }
        _ => {}
    }
    h.finish()
}

// ---------------------------------------------------------------------------
// Session entry point
// ---------------------------------------------------------------------------

/// Runs one quiz WebSocket session and guarantees every task spawned during
/// it (the write task, delayed-answer tasks, …) is cleaned up before
/// returning — regardless of whether the session ended normally or errored.
pub(super) async fn run_quiz_session(
    http: &Client,
    socket_url: &str,
    access_token: &str,
    answer_key: &Arc<RwLock<HashMap<String, u32>>>,
    localization: &HashMap<String, String>,
    config: &QuizConfig,
    tx: &mpsc::UnboundedSender<QuizEvent>,
    stats: &QuizStats,
    connected_once: &AtomicBool,
    origin: &str,
    region: &str,
    consecutive_zero_rounds: &mut u32,
) -> std::result::Result<SessionOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let mut pending: JoinSet<()> = JoinSet::new();

    let result = run_quiz_session_inner(
        http, socket_url, access_token,
        answer_key, localization, config, tx, stats,
        connected_once, &mut pending, origin, region,
        consecutive_zero_rounds,
    )
    .await;

    // Whatever happened above, don't leak the write task or any in-flight
    // delayed-answer tasks into the next reconnect attempt.
    pending.abort_all();
    while pending.join_next().await.is_some() {}

    result
}

/// Cancels a pending delayed-answer submission, if there is one.
///
/// Used whenever a new question arrives before we've submitted an answer to
/// the previous one — the old submission is no longer relevant and should
/// not fire late.
fn cancel_pending(
    cancel_tx: &mut Option<oneshot::Sender<()>>,
    answer_rx: &mut Option<oneshot::Receiver<u32>>,
) {
    if let Some(t) = cancel_tx.take() {
        let _ = t.send(());
    }
    *answer_rx = None;
}

async fn run_quiz_session_inner(
    http: &Client,
    socket_url: &str,
    access_token: &str,
    answer_key: &Arc<RwLock<HashMap<String, u32>>>,
    localization: &HashMap<String, String>,
    config: &QuizConfig,
    tx: &mpsc::UnboundedSender<QuizEvent>,
    stats: &QuizStats,
    connected_once: &AtomicBool,
    pending_tasks: &mut JoinSet<()>,
    origin: &str,
    region: &str,
    consecutive_zero_rounds: &mut u32,
) -> std::result::Result<SessionOutcome, Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!(%socket_url, "Connecting to quiz WebSocket…");

    let ws = http.websocket(socket_url).header("origin", origin).send().await?;
    let (mut write, mut read) = ws.into_websocket().await?.split();
    let mut dedup = EventDeduplicator::new(DEDUP_TTL);

    // -- Handshake -------------------------------------------------------
    quiz_recv_text(&mut read, "EIO handshake", config.handshake_timeout()).await?;
    connected_once.store(true, Ordering::Relaxed);

    let culture = region_to_culture(region);
    write.send(Message::text(format!(
        r#"40{{"jwt":"{access_token}","culture":"{culture}"}}"#
    ))).await?;

    quiz_recv_text(&mut read, "socket.io connected", config.handshake_timeout()).await?;
    write.send(Message::text(r#"42["1000"]"#)).await?;

    let join_response = quiz_recv_text(&mut read, "join confirmation", config.handshake_timeout()).await?;
    tracing::info!("Quiz room joined.");

    let mut active_question: Option<String> = None;

    if let Some(mut event) = parse_quiz_frame(&join_response) {
        if let QuizEvent::Init(ref mut ev) = event {
            ev.expected_answer = answer_key.read().await.get(&ev.question).copied();
            enrich(ev, localization);
            stats.questions_seen.fetch_add(1, Ordering::Relaxed);
            active_question = Some(ev.question.clone());
        }
        let sig = event_signature(&event);
        dedup.is_duplicate(sig);
        if tx.send(event).is_err() {
            return Ok(SessionOutcome::ReceiverGone);
        }
    }

    // Outgoing frames go through a dedicated task + channel rather than
    // being sent directly from the select loop below, so that delayed
    // answer submissions (spawned onto `pending_tasks`) can write to the
    // socket without needing mutable access to `write` themselves.
    let (ws_tx, mut ws_rx) = mpsc::channel::<Message>(WS_WRITE_BUFFER);
    pending_tasks.spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            if write.send(msg).await.is_err() {
                break;
            }
        }
    });

    let mut watchdog = tokio::time::interval(config.watchdog_interval());
    watchdog.tick().await;
    let mut last_rx = Instant::now();
    let mut pending_question: Option<String> = None;
    let mut last_submitted: Option<u32> = None;
    let mut pending_answer_rx: Option<oneshot::Receiver<u32>> = None;
    let mut pending_cancel_tx: Option<oneshot::Sender<()>> = None;

    loop {
        // Reap finished delayed-answer / chat-send tasks so errors in them
        // get logged instead of silently disappearing.
        while let Some(res) = pending_tasks.try_join_next() {
            if let Err(e) = res {
                if !e.is_cancelled() {
                    tracing::warn!(error = %e, "Quiz sub-task ended unexpectedly.");
                }
            }
        }

        tokio::select! {
            _ = watchdog.tick() => {
                if last_rx.elapsed() > config.read_timeout() {
                    tracing::warn!("Quiz inactivity timeout — reconnecting.");
                    return Ok(SessionOutcome::Reconnect);
                }
            }

            confirmed = async {
                match pending_answer_rx.as_mut() {
                    Some(rx) => rx.await.ok(),
                    None     => std::future::pending().await,
                }
            }, if pending_answer_rx.is_some() => {
                pending_answer_rx = None;
                pending_cancel_tx = None;
                if let Some(a) = confirmed {
                    last_submitted = Some(a);
                    stats.answers_submitted.fetch_add(1, Ordering::Relaxed);
                }
            }

            msg = read.next() => {
                let msg = match msg {
                    Some(Ok(m))  => m,
                    Some(Err(e)) => return Err(Box::new(e)),
                    None         => return Ok(SessionOutcome::Reconnect),
                };

                match msg {
                    Message::Text(raw) => {
                        last_rx  = Instant::now();
                        let text = raw.to_string();
                        tracing::debug!(frame = %text, "Quiz frame.");

                        if text == "2" {
                            if ws_tx.send(Message::text("3")).await.is_err() {
                                return Ok(SessionOutcome::Reconnect);
                            }
                            continue;
                        }

                        let Some(mut event) = parse_quiz_frame(&text) else { continue; };

                        match &mut event {
                            QuizEvent::Init(ev) => {
                                ev.expected_answer = answer_key.read().await.get(&ev.question).copied();
                                enrich(ev, localization);
                                stats.questions_seen.fetch_add(1, Ordering::Relaxed);
                            }
                            QuizEvent::QuestionShown(ev) => {
                                ev.expected_answer = answer_key.read().await.get(&ev.question).copied();
                                enrich(ev, localization);
                                stats.questions_seen.fetch_add(1, Ordering::Relaxed);
                            }
                            _ => {}
                        }

                        let sig = event_signature(&event);
                        if dedup.is_duplicate(sig) { continue; }

                        match &event {
                            QuizEvent::Init(ev) => {
                                cancel_pending(&mut pending_cancel_tx, &mut pending_answer_rx);
                                pending_question = Some(ev.question.clone());
                                active_question  = Some(ev.question.clone());
                                last_submitted   = None;
                            }
                            QuizEvent::QuestionShown(ev) => {
                                cancel_pending(&mut pending_cancel_tx, &mut pending_answer_rx);
                                pending_question = Some(ev.question.clone());
                                active_question  = Some(ev.question.clone());
                                last_submitted   = None;

                                if config.send_to_chat {
                                    if let Some(correct_text) = &ev.translated_expected_answer {
                                        let ws_tx2 = ws_tx.clone();
                                        let text   = correct_text.clone();
                                        let delay  = config.chat_answer_delay();
                                        pending_tasks.spawn(async move {
                                            tokio::time::sleep(delay).await;
                                            let frame = format!(
                                                "42{}",
                                                json!(["chatv2:send", { "message": text }])
                                            );
                                            let _ = ws_tx2.send(Message::text(frame)).await;
                                        });
                                    }
                                }
                            }
                            QuizEvent::AnswerWaiting(_) => {
                                if let Some(ref q) = pending_question {
                                    let answer = {
                                        let guard = answer_key.read().await;
                                        pick_answer(q, &guard, config.success_rate)
                                    };
                                    let delay  = config.answer_submit_delay();
                                    let ws_tx2 = ws_tx.clone();
                                    let q_copy = q.clone();
                                    let (ctx, crx)   = oneshot::channel::<u32>();
                                    let (ctx2, crx2) = oneshot::channel::<()>();
                                    pending_answer_rx = Some(crx);
                                    pending_cancel_tx = Some(ctx2);
                                    pending_tasks.spawn(async move {
                                        tokio::select! {
                                            biased;
                                            _ = crx2 => {}
                                            _ = tokio::time::sleep(delay) => {
                                                let frame = format!(
                                                    r#"42["quiz:answer",{{"answer":{answer}}}]"#
                                                );
                                                if ws_tx2.send(Message::text(frame)).await.is_ok() {
                                                    tracing::info!(q = %q_copy, answer, "Submitted.");
                                                    let _ = ctx.send(answer);
                                                }
                                            }
                                        }
                                    });
                                }
                                pending_question = None;
                            }
                            QuizEvent::Reveal(ev) => {
                                if let Some(ref qk) = active_question {
                                    if config.automatically_learn {
                                        let mut updated = false;
                                        {
                                            let mut guard = answer_key.write().await;
                                            if guard.get(qk).copied() != Some(ev.correct_answer) {
                                                guard.insert(qk.clone(), ev.correct_answer);
                                                updated = true;
                                            }
                                        }
                                        if updated {
                                            if let Some(ref path) = config.custom_questions_path {
                                                let map = answer_key.read().await.clone();
                                                if let Ok(s) = serde_json::to_string_pretty(&map) {
                                                    let p = path.clone();
                                                    tokio::spawn(async move {
                                                        if let Some(parent) = p.parent() {
                                                            let _ = tokio::fs::create_dir_all(parent).await;
                                                        }
                                                        let _ = tokio::fs::write(p, s).await;
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                                if let Some(submitted) = last_submitted {
                                    if submitted == ev.correct_answer {
                                        stats.correct_answers.fetch_add(1, Ordering::Relaxed);
                                        tracing::info!(answer = %ev.correct_answer, "✓ Correct.");
                                    } else {
                                        tracing::info!(
                                            submitted = %submitted,
                                            correct   = %ev.correct_answer,
                                            "✗ Wrong."
                                        );
                                    }
                                }
                                let snap = stats.snapshot();
                                tracing::info!(
                                    seen      = snap.questions_seen,
                                    submitted = snap.answers_submitted,
                                    correct   = snap.correct_answers,
                                    "Running stats."
                                );
                                last_submitted  = None;
                                active_question = None;
                            }

                            // ── Daily-limit detection ─────────────────────
                            QuizEvent::RoundEnd(ev) => {
                                let round_xp: u64 = ev.reward_parts.iter().map(|p| p.xp as u64).sum();
                                let round_sc: u64 = ev.reward_parts.iter().map(|p| p.soft_currency as u64).sum();

                                if round_xp == 0 && round_sc == 0 {
                                    *consecutive_zero_rounds += 1;
                                    tracing::warn!(
                                        count = *consecutive_zero_rounds,
                                        threshold = DAILY_LIMIT_ZERO_ROUNDS_THRESHOLD,
                                        "Round ended with XP=0 and SC=0 — possible daily limit."
                                    );

                                    if *consecutive_zero_rounds >= DAILY_LIMIT_ZERO_ROUNDS_THRESHOLD {
                                        // Forward the event to the consumer before
                                        // returning so it can display the final
                                        // round-end stats.
                                        let _ = tx.send(event);
                                        return Ok(SessionOutcome::DailyLimitReached);
                                    }
                                } else {
                                    // A non-zero round resets the streak.
                                    *consecutive_zero_rounds = 0;
                                }
                            }

                            QuizEvent::NewGameReady(_) => {
                                if ws_tx.send(Message::text(r#"42["quiz:newgameready"]"#)).await.is_err() {
                                    return Ok(SessionOutcome::Reconnect);
                                }
                            }
                            _ => {}
                        }

                        if tx.send(event).is_err() {
                            return Ok(SessionOutcome::ReceiverGone);
                        }
                    }
                    Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => {
                        last_rx = Instant::now();
                    }
                    Message::Close(_) => {
                        tracing::warn!("Quiz socket closed.");
                        return Ok(SessionOutcome::Reconnect);
                    }
                }
            }
        }
    }
}

/// Waits for the next text frame during the handshake sequence, timing out
/// after `timeout`. Non-text frames received during the handshake are
/// treated as a no-op (empty string) rather than an error.
async fn quiz_recv_text<S, E>(
    read: &mut S,
    label: &str,
    timeout: Duration,
) -> std::result::Result<String, Box<dyn std::error::Error + Send + Sync>>
where
    S: StreamExt<Item = std::result::Result<Message, E>> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    match tokio::time::timeout(timeout, read.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => {
            let s = t.to_string();
            tracing::debug!("{label}: {s}");
            Ok(s)
        }
        Ok(Some(Ok(_)))  => Ok(String::new()),
        Ok(Some(Err(e))) => Err(Box::new(e)),
        Ok(None)         => Err(format!("closed awaiting {label}").into()),
        Err(_)           => Err(format!("timeout awaiting {label}").into()),
    }
}