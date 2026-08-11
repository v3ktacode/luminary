use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone)]
pub enum MspEvent {
    PingResponse(PingResponseEvent),
    RelationshipRequestCreated(RelationshipRequestCreatedEvent),
    RelationshipRequestChanged(RelationshipRequestChangedEvent),
    PassiveRewardEarned(PassiveRewardEarnedEvent),
    MessageSent(MessageSentEvent),
    Unknown {
        message_type: String,
        payload:      Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PingResponseEvent {
    pub ping_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipRequestCreatedEvent {
    pub requester_profile_id: String,
    pub profile_id:           String,
    pub game_id:              String,
    pub target_profile_ids:   Vec<String>,
    pub event_name:           String,
    pub event_version:        u32,
    pub trace_parent:         Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipRequestChangedEvent {
    pub created:              Option<String>,
    pub new_state:            String,
    pub old_state:            String,
    pub requester_profile_id: String,
    pub profile_id:           String,
    pub game_id:              String,
    pub target_profile_ids:   Vec<String>,
    pub event_name:           String,
    pub event_version:        u32,
    pub trace_parent:         Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PassiveRewardEarnedEvent {
    pub target_profile_ids: Vec<String>,
    pub profile_id:         String,
    pub game_id:            String,
    pub when:               String,
    pub xp:                 i64,
    pub currency_rewards:   Value,
    pub reward_id:          String,
    pub sub_type:           Option<String>,
    pub vip_days:           i64,
    pub collect:            Option<RewardCollect>,
    pub source_profile_id:  Option<String>,
    pub event_name:         String,
    pub event_version:      u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardCollect {
    pub group_id: String,
    pub guid:     String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSentEvent {
    pub author:             String,
    pub sender_profile_id:  String,
    pub conversation_id:    String,
    pub conversation_name:  String,
    pub conversation_type:  String,
    pub message_body:       String,
    pub message_id:         String,
    pub message_type:       String,
    pub message_version:    u32,
    pub muted_profile_ids:  Vec<String>,
    pub target_profile_ids: Vec<String>,
    pub timestamp:          String,
    pub event_name:         String,
    pub event_version:      u32,
    pub trace_parent:       Option<String>,
}

// ─── Quiz Room Events ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuizInitEvent {
    pub current_turn:                    u32,
    pub turns_per_round:                 u32,
    pub time_to_answer_in_seconds:       u32,
    pub current_round:                   u32,
    pub rounds_per_game:                 u32,
    pub state:                           String,
    pub player_scores:                   Vec<PlayerScore>,
    pub game_end_state_duration_seconds: u32,
    pub question:                        String,
    pub answers:                         Vec<String>,
    #[serde(skip)]
    pub expected_answer:                 Option<u32>,
    #[serde(skip)]
    pub translated_question:             Option<String>,
    #[serde(skip)]
    pub translated_answers:              Option<Vec<String>>,
    #[serde(skip)]
    pub translated_expected_answer:      Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerScore {
    pub profile_id: String,
    pub score:      i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuizPlayerJoinedEvent {
    pub profile_id:   String,
    pub session_id:   u64,
    pub profile_data: JoinedProfileData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinedProfileData {
    pub name:   String,
    pub is_vip: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuizAnswerWaitingEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuizPlayerVotedEvent {
    pub session_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuizQuestionShownEvent {
    pub question: String,
    pub answers:  Vec<String>,
    #[serde(skip)]
    pub expected_answer:             Option<u32>,
    #[serde(skip)]
    pub translated_question:         Option<String>,
    #[serde(skip)]
    pub translated_answers:          Option<Vec<String>>,
    #[serde(skip)]
    pub translated_expected_answer:  Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuizRevealEvent {
    pub correct_answer: u32,
    pub winners:        Vec<String>,
    pub player_scores:  Vec<PlayerScore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuizRoundEndEvent {
    pub player_round_scores: Vec<PlayerRoundScore>,
    pub reward_parts:        Vec<RewardPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRoundScore {
    pub profile_id:                 String,
    pub total_score:                i64,
    pub round_score:                i64,
    pub correct_answers_this_round: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardPart {
    pub xp:            f64,
    pub soft_currency: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuizGameEndEvent {
    pub player_scores: Vec<PlayerScore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuizNewGameReadyEvent {
    pub session_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuizTurnStartStateEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuizTurnEndEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuizRoundStartStateEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuizRoundEndStateEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuizShowChallengeStateEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuizGameEndStateEvent;

// ─── Quiz event enum ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum QuizEvent {
    Init(QuizInitEvent),
    PlayerJoined(QuizPlayerJoinedEvent),
    AnswerWaiting(QuizAnswerWaitingEvent),
    PlayerVoted(QuizPlayerVotedEvent),
    QuestionShown(QuizQuestionShownEvent),
    Reveal(QuizRevealEvent),
    RoundEnd(QuizRoundEndEvent),
    GameEnd(QuizGameEndEvent),
    NewGameReady(QuizNewGameReadyEvent),
    TurnStartState(QuizTurnStartStateEvent),
    TurnEnd(QuizTurnEndEvent),
    RoundStartState(QuizRoundStartStateEvent),
    RoundEndState(QuizRoundEndStateEvent),
    ShowChallengeState(QuizShowChallengeStateEvent),
    GameEndState(QuizGameEndStateEvent),
    /// Synthetic event emitted by the supervisor when it detects that two or
    /// more consecutive rounds returned XP = 0 **and** SC = 0, which is the
    /// server's signal that the daily reward cap has been reached.
    DailyLimitReached,
    Unknown {
        event_type: String,
        payload:    Value,
    },
}

// ─── Presence frame parser ────────────────────────────────────────────────────

pub(crate) fn parse_frame(text: &str) -> Option<MspEvent> {
    if !text.starts_with("42") {
        return None;
    }

    let json_part = &text[2..];
    let array: Value = serde_json::from_str(json_part).ok()?;

    let inner: Value = match array.get(1)? {
        Value::String(s) => serde_json::from_str(s).ok()?,
        Value::Object(_) => array.get(1)?.clone(),
        _ => return None,
    };

    let message_type = inner["messageType"].as_str().unwrap_or("").to_owned();
    let content      = &inner["messageContent"];

    match message_type.as_str() {
        "501" => {
            let event: PingResponseEvent =
                serde_json::from_value(content.clone()).ok()?;
            Some(MspEvent::PingResponse(event))
        }

        "100" => {
            let event_name = content["eventName"].as_str().unwrap_or("");
            match event_name {
                "relationshipRequestCreatedEvent" => {
                    let event: RelationshipRequestCreatedEvent =
                        serde_json::from_value(content.clone()).ok()?;
                    Some(MspEvent::RelationshipRequestCreated(event))
                }
                "relationshipRequestChangedEvent" => {
                    let event: RelationshipRequestChangedEvent =
                        serde_json::from_value(content.clone()).ok()?;
                    Some(MspEvent::RelationshipRequestChanged(event))
                }
                "passiveRewardEarnedEvent" => {
                    let event: PassiveRewardEarnedEvent =
                        serde_json::from_value(content.clone()).ok()?;
                    Some(MspEvent::PassiveRewardEarned(event))
                }
                "messageSentEvent" => {
                    let event: MessageSentEvent =
                        serde_json::from_value(content.clone()).ok()?;
                    Some(MspEvent::MessageSent(event))
                }
                _ => Some(MspEvent::Unknown {
                    message_type,
                    payload: inner.clone(),
                }),
            }
        }

        _ => Some(MspEvent::Unknown {
            message_type,
            payload: inner.clone(),
        }),
    }
}

// ─── Quiz frame parser ────────────────────────────────────────────────────────

pub(crate) fn parse_quiz_frame(text: &str) -> Option<QuizEvent> {
    if !text.starts_with("42") {
        return None;
    }

    let array: Value = serde_json::from_str(&text[2..]).ok()?;
    let event_name   = array.get(0)?.as_str()?.to_owned();
    let payload      = array.get(1)?.clone();

    match event_name.as_str() {
        "quiz:init" => {
            let ev: QuizInitEvent = serde_json::from_value(payload).ok()?;
            Some(QuizEvent::Init(ev))
        }
        "quiz:chal" => {
            let ev: QuizQuestionShownEvent = serde_json::from_value(payload).ok()?;
            Some(QuizEvent::QuestionShown(ev))
        }
        "quiz:answer" => {
            let ev: QuizPlayerVotedEvent = serde_json::from_value(payload).ok()?;
            Some(QuizEvent::PlayerVoted(ev))
        }
        "quiz:reveal" => {
            let ev: QuizRevealEvent = serde_json::from_value(payload).ok()?;
            Some(QuizEvent::Reveal(ev))
        }
        "quiz:roundend" => {
            let ev: QuizRoundEndEvent = serde_json::from_value(payload).ok()?;
            Some(QuizEvent::RoundEnd(ev))
        }
        "quiz:gameend" => {
            let ev: QuizGameEndEvent = serde_json::from_value(payload).ok()?;
            Some(QuizEvent::GameEnd(ev))
        }
        "quiz:newgameready" => {
            let ev: QuizNewGameReadyEvent = serde_json::from_value(payload).ok()?;
            Some(QuizEvent::NewGameReady(ev))
        }
        "20000" => {
            let ev: QuizPlayerJoinedEvent = serde_json::from_value(payload).ok()?;
            Some(QuizEvent::PlayerJoined(ev))
        }
        "game:state" => {
            let new_state = payload["newState"].as_str().unwrap_or("").to_owned();
            parse_game_state(&new_state, payload)
        }
        "message" => {
            let msg_type = payload["messageType"].as_str().unwrap_or("").to_owned();
            parse_message_frame(msg_type, payload)
        }
        _ => Some(QuizEvent::Unknown {
            event_type: event_name,
            payload,
        }),
    }
}

fn parse_game_state(new_state: &str, payload: Value) -> Option<QuizEvent> {
    match new_state {
        "round_start"        => Some(QuizEvent::RoundStartState(QuizRoundStartStateEvent)),
        "turn_start"         => Some(QuizEvent::TurnStartState(QuizTurnStartStateEvent)),
        "show_challenge"     => Some(QuizEvent::ShowChallengeState(QuizShowChallengeStateEvent)),
        "waiting_for_answer" => Some(QuizEvent::AnswerWaiting(QuizAnswerWaitingEvent)),
        "turn_end"           => Some(QuizEvent::TurnEnd(QuizTurnEndEvent)),
        "round_end"          => Some(QuizEvent::RoundEndState(QuizRoundEndStateEvent)),
        "game_end"           => Some(QuizEvent::GameEndState(QuizGameEndStateEvent)),
        _ => Some(QuizEvent::Unknown {
            event_type: format!("game:state:{new_state}"),
            payload,
        }),
    }
}

fn parse_message_frame(msg_type: String, payload: Value) -> Option<QuizEvent> {
    let content = &payload["messageContent"];
    match msg_type.as_str() {
        "2000" => None,
        "quiz:init" => {
            let ev: QuizInitEvent = serde_json::from_value(content.clone()).ok()?;
            Some(QuizEvent::Init(ev))
        }
        "quiz:chal" => {
            let ev: QuizQuestionShownEvent = serde_json::from_value(content.clone()).ok()?;
            Some(QuizEvent::QuestionShown(ev))
        }
        "game:state" => {
            let new_state = content["newState"].as_str().unwrap_or("").to_owned();
            parse_game_state(&new_state, payload)
        }
        "quiz:answer" => {
            let ev: QuizPlayerVotedEvent = serde_json::from_value(content.clone()).ok()?;
            Some(QuizEvent::PlayerVoted(ev))
        }
        "quiz:reveal" => {
            let ev: QuizRevealEvent = serde_json::from_value(content.clone()).ok()?;
            Some(QuizEvent::Reveal(ev))
        }
        "quiz:roundend" => {
            let ev: QuizRoundEndEvent = serde_json::from_value(content.clone()).ok()?;
            Some(QuizEvent::RoundEnd(ev))
        }
        "quiz:gameend" => {
            let ev: QuizGameEndEvent = serde_json::from_value(content.clone()).ok()?;
            Some(QuizEvent::GameEnd(ev))
        }
        "quiz:newgameready" => {
            let ev: QuizNewGameReadyEvent = serde_json::from_value(content.clone()).ok()?;
            Some(QuizEvent::NewGameReady(ev))
        }
        "20000" => {
            let ev: QuizPlayerJoinedEvent = serde_json::from_value(content.clone()).ok()?;
            Some(QuizEvent::PlayerJoined(ev))
        }
        _ => Some(QuizEvent::Unknown {
            event_type: msg_type,
            payload,
        }),
    }
}