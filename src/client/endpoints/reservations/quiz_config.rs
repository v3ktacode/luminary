// src/client/endpoints/reservations/quiz_config.rs
//
// Tuning knobs and runtime counters for `play_star_quiz` / `play_star_quiz_ex`.
//
// `QuizConfig` is the public, user-facing builder. `QuizStats` is the
// internal counter set the supervisor updates as it runs; `QuizStatsSnapshot`
// is a plain-data copy handed out to callers (the atomics inside `QuizStats`
// aren't `Clone`, so a snapshot type is needed for anything that wants to
// display or serialize the current counts).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;

#[derive(Debug, Clone)]
pub struct QuizConfig {
    pub success_rate: f64,
    pub send_to_chat: bool,
    pub answer_submit_delay_ms: u64,
    pub chat_answer_delay_ms: u64,
    pub play_forever: bool,
    pub reconnect_extra_delay_ms: u64,
    pub initial_backoff_secs: u64,
    pub max_backoff_secs: u64,
    pub jitter_max_ms: u64,
    pub handshake_timeout_secs: u64,
    pub read_timeout_secs: u64,
    pub watchdog_interval_secs: u64,
    pub custom_questions_path: Option<PathBuf>,
    pub automatically_learn: bool,
}

impl Default for QuizConfig {
    fn default() -> Self {
        Self {
            success_rate: 1.0,
            send_to_chat: false,
            answer_submit_delay_ms: 500,
            chat_answer_delay_ms: 1500,
            play_forever: true,
            reconnect_extra_delay_ms: 0,
            initial_backoff_secs: 2,
            max_backoff_secs: 60,
            jitter_max_ms: 500,
            handshake_timeout_secs: 20,
            read_timeout_secs: 45,
            watchdog_interval_secs: 5,
            custom_questions_path: None,
            automatically_learn: true,
        }
    }
}

impl QuizConfig {
    pub fn success_rate(mut self, v: f64) -> Self { self.success_rate = v.clamp(0.0, 1.0); self }
    pub fn send_to_chat(mut self, v: bool) -> Self { self.send_to_chat = v; self }
    pub fn answer_submit_delay_ms(mut self, v: u64) -> Self { self.answer_submit_delay_ms = v; self }
    pub fn chat_answer_delay_ms(mut self, v: u64) -> Self { self.chat_answer_delay_ms = v; self }
    pub fn play_forever(mut self, v: bool) -> Self { self.play_forever = v; self }
    pub fn reconnect_extra_delay_ms(mut self, v: u64) -> Self { self.reconnect_extra_delay_ms = v; self }
    pub fn initial_backoff_secs(mut self, v: u64) -> Self { self.initial_backoff_secs = v; self }
    pub fn max_backoff_secs(mut self, v: u64) -> Self { self.max_backoff_secs = v; self }
    pub fn jitter_max_ms(mut self, v: u64) -> Self { self.jitter_max_ms = v; self }
    pub fn handshake_timeout_secs(mut self, v: u64) -> Self { self.handshake_timeout_secs = v; self }
    pub fn read_timeout_secs(mut self, v: u64) -> Self { self.read_timeout_secs = v; self }
    pub fn watchdog_interval_secs(mut self, v: u64) -> Self { self.watchdog_interval_secs = v; self }
    pub fn custom_questions_path(mut self, v: Option<impl Into<PathBuf>>) -> Self {
        self.custom_questions_path = v.map(|p| p.into()); self
    }
    pub fn automatically_learn(mut self, v: bool) -> Self { self.automatically_learn = v; self }

    pub(super) fn handshake_timeout(&self) -> Duration { Duration::from_secs(self.handshake_timeout_secs) }
    pub(super) fn read_timeout(&self) -> Duration { Duration::from_secs(self.read_timeout_secs) }
    pub(super) fn watchdog_interval(&self) -> Duration { Duration::from_secs(self.watchdog_interval_secs) }
    pub(super) fn answer_submit_delay(&self) -> Duration { Duration::from_millis(self.answer_submit_delay_ms) }
    pub(super) fn chat_answer_delay(&self) -> Duration { Duration::from_millis(self.chat_answer_delay_ms) }
    pub(super) fn reconnect_extra_delay(&self) -> Duration { Duration::from_millis(self.reconnect_extra_delay_ms) }
    pub(super) fn initial_backoff(&self) -> Duration { Duration::from_secs(self.initial_backoff_secs) }
    pub(super) fn max_backoff(&self) -> Duration { Duration::from_secs(self.max_backoff_secs) }
}

#[derive(Debug, Default)]
pub struct QuizStats {
    pub sessions_completed: AtomicU64,
    pub sessions_error: AtomicU64,
    pub total_reconnects: AtomicU64,
    pub questions_seen: AtomicU64,
    pub answers_submitted: AtomicU64,
    pub correct_answers: AtomicU64,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuizStatsSnapshot {
    pub sessions_completed: u64,
    pub sessions_error: u64,
    pub total_reconnects: u64,
    pub questions_seen: u64,
    pub answers_submitted: u64,
    pub correct_answers: u64,
}

impl QuizStats {
    pub fn snapshot(&self) -> QuizStatsSnapshot {
        QuizStatsSnapshot {
            sessions_completed: self.sessions_completed.load(Ordering::Relaxed),
            sessions_error: self.sessions_error.load(Ordering::Relaxed),
            total_reconnects: self.total_reconnects.load(Ordering::Relaxed),
            questions_seen: self.questions_seen.load(Ordering::Relaxed),
            answers_submitted: self.answers_submitted.load(Ordering::Relaxed),
            correct_answers: self.correct_answers.load(Ordering::Relaxed),
        }
    }
}