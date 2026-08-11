// src/client/endpoints/reservations/mod.rs
//
// Room reservations and the "Star Quiz" automation built on top of one of
// them.
//
// A "reservation" is how the game finds/creates a multiplayer room instance
// — chatrooms and the quiz minigame are the two room types this client
// knows how to reserve (`RoomKind`). `chatroom()` / `quiz()` just make the
// reservation; `play_star_quiz` / `play_star_quiz_ex` go a step further and
// actually connect to and play the quiz room autonomously.
//
// This module is split into several files:
//
//   mod.rs           — this file: the endpoint struct and its public API.
//   region.rs        — region → locale lookup tables.
//   quiz_config.rs   — `QuizConfig` (tuning knobs) and the stats types.
//   localization.rs  — quiz question/answer translation.
//   answer_key.rs    — loading/caching the question → correct-answer map.
//   supervisor.rs    — the reconnect loop around a quiz session.
//   session.rs       — a single quiz WebSocket connection end-to-end.

mod answer_key;
mod localization;
mod quiz_config;
mod region;
mod session;
mod supervisor;

pub use quiz_config::{QuizConfig, QuizStats, QuizStatsSnapshot};

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::{mpsc, RwLock};
use wreq::Client;

use super::super::http::{build_headers, decode_response_value, ContentType};
use crate::{
    config::MspConfig,
    errors::{MspError, Result},
    events::QuizEvent,
    models::{RoomKind, RoomReservation},
    session::SessionStore,
};
use answer_key::load_answer_key;
use localization::fetch_localization;
use region::{region_to_culture, region_to_lang_code};
use supervisor::quiz_supervisor;

/// Short label attached to every `MspError` raised from this module.
const EP: &str = "reservations";

pub struct ReservationsEndpoint<'c> {
    pub(crate) http: &'c Client,
    pub(crate) session: &'c SessionStore,
    pub(crate) config: Arc<MspConfig>,
}

impl<'c> ReservationsEndpoint<'c> {
    /// Reserves a chatroom instance at the given level/version.
    #[tracing::instrument(name = "reservations.chatroom", skip(self),
        fields(level = %level, version = %version))]
    pub async fn chatroom(&self, level: &str, version: &str) -> Result<RoomReservation> {
        self.reserve(RoomKind::Chatroom, level, version).await
    }

    /// Reserves a Star Quiz room instance.
    ///
    /// `"624"` is the quiz asset version the game currently ships — there's
    /// no discovery mechanism for this, it's just what the client sends.
    #[tracing::instrument(name = "reservations.quiz", skip(self))]
    pub async fn quiz(&self) -> Result<RoomReservation> {
        self.reserve(RoomKind::Quiz, "", "624").await
    }

    /// Reserves a quiz room, connects to it, and plays it autonomously,
    /// answering questions with the given `success_rate` (0.0–1.0) and
    /// optionally posting the correct answer in chat.
    ///
    /// Returns a channel of `QuizEvent`s so the caller can observe what's
    /// happening (question shown, answer submitted, round ended, …) without
    /// blocking. For finer control over timing/back-off/persistence, use
    /// `play_star_quiz_ex` with a custom `QuizConfig` instead.
    pub async fn play_star_quiz(
        &self,
        success_rate: f64,
        send_to_chat: bool,
    ) -> Result<mpsc::UnboundedReceiver<QuizEvent>> {
        self.play_star_quiz_ex(
            QuizConfig::default()
                .success_rate(success_rate)
                .send_to_chat(send_to_chat),
        )
        .await
    }

    /// Same as `play_star_quiz`, but with full control over behaviour via
    /// `QuizConfig` (custom answer cache path, back-off tuning, whether to
    /// keep reconnecting forever, …).
    ///
    /// The quiz session runs on a spawned background task; this method
    /// returns as soon as the room is reserved and the answer key /
    /// translations are loaded, without waiting for the quiz to finish.
    ///
    /// **Important:** if the automation detects the daily reward cap has
    /// been hit, it currently terminates the *entire process* via
    /// `std::process::exit` (after emitting a final `QuizEvent::DailyLimitReached`
    /// on the returned channel) — not just this background task. This is a
    /// deliberate but blunt design choice inherited from the original
    /// implementation; worth knowing if you're embedding this client inside
    /// a larger long-running application.
    #[tracing::instrument(name = "reservations.play_star_quiz_ex", skip(self))]
    pub async fn play_star_quiz_ex(
        &self,
        config: QuizConfig,
    ) -> Result<mpsc::UnboundedReceiver<QuizEvent>> {
        let reservation = self.quiz().await?;
        tracing::info!(
            room_id = %reservation.room_id,
            socket_url = %reservation.socket_url,
            "Quiz room reserved."
        );

        let answer_key_map = load_answer_key(&config, self.http).await.unwrap_or_else(|e| {
            tracing::warn!("Could not fetch answer key ({e:?}); will answer randomly.");
            std::collections::HashMap::new()
        });
        let answer_key = Arc::new(RwLock::new(answer_key_map));

        let session = self.session.get().await?;
        let access_token = session.access_token.clone();
        let region = session.region.clone();

        let lang_code = region_to_lang_code(&region);
        let localization = fetch_localization(self.http, lang_code).await.unwrap_or_else(|e| {
            tracing::warn!("Could not fetch localization ({e:?}); translations disabled.");
            std::collections::HashMap::new()
        });

        let http = self.http.clone();
        let origin = self.config.origin.clone();
        let socket_url = reservation.socket_url.clone();
        let (tx, rx) = mpsc::unbounded_channel::<QuizEvent>();

        tokio::spawn(async move {
            quiz_supervisor(
                &http, &socket_url, &access_token,
                &answer_key, &localization, &config, &tx, &origin, &region,
            )
            .await;
        });

        Ok(rx)
    }

    /// Shared implementation behind `chatroom()` and `quiz()` — sends the
    /// `FindRoomByType` reservation request and turns the response into a
    /// `RoomReservation`, including building the eventual WebSocket URL
    /// (which differs from `hostUrl` by room kind — see
    /// `RoomKind::eio_version` / `RoomKind::socket_path`).
    async fn reserve(&self, kind: RoomKind, level: &str, version: &str) -> Result<RoomReservation> {
        let session = self.session.get().await?;
        let culture = region_to_culture(&session.region);

        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Parameters<'a> {
            #[serde(rename = "LoadMode")] load_mode: &'a str,
            #[serde(rename = "Level")] level: &'a str,
            #[serde(rename = "Version")] version: &'a str,
            #[serde(rename = "Culture")] culture: &'a str,
        }

        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Payload<'a> {
            join_type: &'a str,
            room_type: &'a str,
            room_instance_id: Option<()>,
            parameters: Parameters<'a>,
        }

        let payload = Payload {
            join_type: "FindRoomByType",
            room_type: kind.as_str(),
            room_instance_id: None,
            parameters: Parameters {
                load_mode: "Asset",
                level,
                version,
                culture,
            },
        };

        let response = self
            .http
            .post(self.config.reservations_regional(&session.region))
            .headers(build_headers(
                ContentType::Json,
                Some(&session.bearer()),
                &self.config.origin,
                &self.config.referer,
            ))
            .json(&payload)
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        let value = decode_response_value(response, EP).await?;

        let host_url = value["hostUrl"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| MspError::api(EP, 422, "Missing 'hostUrl' in reservation response"))?
            .to_owned();

        let room_id = value["roomId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| MspError::api(EP, 422, "Missing 'roomId' in reservation response"))?
            .to_owned();

        let socket_url = format!(
            "{}{path}?EIO={eio}&transport=websocket",
            host_url,
            path = kind.socket_path(),
            eio = kind.eio_version(),
        );

        Ok(RoomReservation { host_url, room_id, socket_url })
    }
}