# Luminary

> **Luminary** is a comprehensive, asynchronous API wrapper for **MovieStarPlanet 2**. As the largest and most complete implementation for the platform, it handles session management, browser emulation, and transport-level consistency.

---

## Overview

Luminary provides a structured client for MovieStarPlanet 2. Rather than leaving TLS, HTTP/2, headers, cookies, and session state to generic defaults, it keeps these layers aligned within one client lifecycle.

| Layer | Focus | Implementation |
| :--- | :--- | :--- |
| **Transport** | TLS fidelity | Browser-like TLS handshakes, including JA3/JA4-related characteristics, through BoringSSL. |
| **Protocol** | HTTP/2 framing | Browser-oriented HTTP/2 settings, window sizing, and pseudo-header order. |
| **Application** | Header consistency | Coherent request headers, locale, and `Accept-Language` handling. |
| **Lifecycle** | Session and presence | OAuth2 authentication, persistent cookies, token rotation, and optional real-time presence. |

---

## Quick Start

```rust
use luminary::MspClient;
use wreq_util::{Platform, Profile};

let client = MspClient::builder()
    .profile(Profile::Chrome137)
    .platform(Platform::Windows)
    .locale("fr-FR,fr;q=0.9,en-US;q=0.8,en;q=0.7")
    .presence(true) // Connect to the Presence server after login.
    .build()?;
```

## Table of Contents

- [Builder Configuration Options](#builder-configuration-options)
- [Authentication](#authentication)
  - [Presence Events](#presence-events)
- [Endpoint Reference](#endpoint-reference)
  - [Greetings](#greetings)
  - [Attributes](#attributes)
  - [Reservations](#reservations)
  - [Star Quiz Automation](#star-quiz-automation)
  - [Messaging](#messaging)
- [Core Features](#core-features)
- [Disclaimer](#disclaimer)

---

<a id="builder-configuration-options"></a>
## Builder Configuration Options

`MspClient::builder()` returns an `MspClientBuilder`. Configuration methods consume and return the builder, so they can be chained before calling `build()` or `build_async()`.

<br>

#### `.config(config: MspConfig) -> MspClientBuilder`
Replaces the client configuration and refreshes the timeout values from it.

#### `.device_id(id: impl Into<String>) -> MspClientBuilder`
Generates an uppercase UUID-derived device ID when omitted.

#### `.profile(profile: Profile) -> MspClientBuilder`
Uses `Profile::Chrome137` by default. Selecting a fixed profile disables profile randomisation.

#### `.platform(platform: Platform) -> MspClientBuilder`
Uses `Platform::Windows` by default. Selecting a fixed platform disables platform randomisation.

#### `.random_platform() -> MspClientBuilder`
Selects Windows, macOS, or Linux once when the client is built.

#### `.random_profile(brand: BrowserBrand) -> MspClientBuilder`
Selects one current profile for the chosen browser family (`Chrome`, `Firefox`, or `Any`) when the client is built.

#### `.proxy(proxy: impl Into<String>) -> MspClientBuilder`
No proxy by default. The proxy string is passed to the HTTP client after normalisation.

#### `.enforce_proxy(enforce: bool) -> MspClientBuilder`
`false` by default. When `true`, construction fails unless a proxy has been configured.

#### `.timeout(duration: Duration) -> MspClientBuilder`
Uses the request timeout from `MspConfig` by default.

#### `.connect_timeout(duration: Duration) -> MspClientBuilder`
Uses the connection timeout from `MspConfig` by default.

#### `.locale(locale: impl Into<String>) -> MspClientBuilder`
Overrides `Accept-Language`. A new client otherwise uses the French default; a restored session derives it from its region.

#### `.stealth(min: Duration, max: Duration) -> MspClientBuilder`
Enables random pacing between actions. Pacing is disabled by default; reversed bounds are normalised.

#### `.from_state(state: SessionState) -> MspClientBuilder`
Restores the session, cookies, identity, browser profile, platform, and proxy policy from exported state.

#### `.presence(enabled: bool) -> MspClientBuilder`
`true` by default. Controls whether the Presence WebSocket is started after login.

#### `.build() -> Result<MspClient>`
Builds synchronously. Inside a current-thread Tokio runtime, use `build_async()` instead.

#### `.build_async() -> Future<Output = Result<MspClient>>`
Asynchronously builds the client and is safe to await from Tokio applications.

---

<a id="authentication"></a>
## Authentication

`client.auth().login(username, password, region)` runs the login flow, stores the resulting session, and starts the configured background services. The `region` value is case-insensitive and normalised internally.

| Outcome | Value | Meaning |
| :--- | :--- | :--- |
| Success | `MspSession` | A profile-scoped session containing tokens, the profile ID, device ID, expiry, and region. |
| Invalid credentials | `MspError::InvalidCredentials { username, region }` | The supplied account credentials were rejected. |
| Banned account | `MspError::AccountBanned { username, region, reason }` | The account is permanently restricted; `reason` contains the server-provided context. |
| Other failure | `MspError` | Covers other authentication, transport, or response-processing failures. |

```rust
use luminary::events::MspEvent;

match client.auth().login("username", "password", "FR").await {
    Ok(session) => {
        println!("Successfully authenticated profile ID: {}", session.profile_id);

        // The Presence server pushes events (friend requests, chat messages,
        // reward notifications, heartbeats, …) once connected. Subscribe to
        // the event bus to receive them — any number of subscribers can
        // listen independently.
        let mut events = client.events();
        tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                match event {
                    MspEvent::MessageSent(msg) => {
                        println!("{}: {}", msg.sender_profile_id, msg.message_body);
                    }
                    MspEvent::RelationshipRequestCreated(req) => {
                        println!("Friend request from {}", req.requester_profile_id);
                    }
                    MspEvent::PassiveRewardEarned(reward) => {
                        println!("Earned {} XP ({})", reward.xp, reward.reward_id);
                    }
                    MspEvent::PingResponse(_) => {} // heartbeat ack, usually ignored
                    other => tracing::debug!(?other, "Unhandled presence event."),
                }
            }
        });
    }
    Err(MspError::InvalidCredentials { username, region }) => {
        tracing::error!(%username, %region, "Invalid credentials.");
    }
    Err(MspError::AccountBanned { username, region, reason }) => {
        tracing::error!(%username, %region, %reason, "Account banned.");
    }
    Err(error) => {
        tracing::error!(%error, "Authentication failed.");
    }
}
```

<a id="presence-events"></a>
### Presence Events — `MspEvent`

Once authenticated, the client keeps a persistent WebSocket connection to MovieStarPlanet 2's Presence server open in the background (enabled by default — see [`.presence(...)`](#builder-configuration-options)). Server-pushed events are parsed and published to an internal `EventBus`. `client.events()` is a convenience that subscribes for you and returns a ready-to-use `broadcast::Receiver<MspEvent>` directly — any number of independent calls can each get their own receiver.

#### `EventBus`

The bus itself isn't exposed directly — `client.events()` is the entry point and is equivalent to calling `.subscribe()` on it.

| Method | Returns | Description |
| :--- | :--- | :--- |
| `.subscribe()` | `broadcast::Receiver<MspEvent>` | Returns a new receiver. Each subscriber gets its own copy of every event published after it subscribes. |
| `.subscriber_count()` | `usize` | Number of currently active subscribers. |

The bus is backed by a `tokio::sync::broadcast` channel with a capacity of 256 events. A slow subscriber that falls behind starts missing the oldest buffered events rather than blocking the connection for everyone else.

<br>

**`MspEvent` variants**

| Variant | Payload | Sent when |
| :--- | :--- | :--- |
| `PingResponse` | `PingResponseEvent` | The server echoes back a heartbeat ping ID. |
| `RelationshipRequestCreated` | `RelationshipRequestCreatedEvent` | A friend/relationship request is created. |
| `RelationshipRequestChanged` | `RelationshipRequestChangedEvent` | A friend/relationship request's state changes (accepted, declined, …). |
| `MessageSent` | `MessageSentEvent` | A chat message is sent in a conversation the profile is part of. |
| `PassiveRewardEarned` | `PassiveRewardEarnedEvent` | A passive reward — XP, currency, or VIP days — is earned. |
| `Unknown` | `{ message_type: String, payload: Value }` | Any frame the client doesn't have a typed event for. The raw `messageType` and payload are preserved so nothing is silently dropped. |

<br>

**Event payload fields**

| Event | Key fields |
| :--- | :--- |
| `PingResponseEvent` | `ping_id` |
| `RelationshipRequestCreatedEvent` | `requester_profile_id`, `profile_id`, `game_id`, `target_profile_ids`, `event_name`, `event_version`, `trace_parent` |
| `RelationshipRequestChangedEvent` | Everything `RelationshipRequestCreatedEvent` has, plus `created`, `new_state`, `old_state` |
| `PassiveRewardEarnedEvent` | `profile_id`, `game_id`, `when`, `xp`, `currency_rewards`, `reward_id`, `sub_type`, `vip_days`, `collect: Option<RewardCollect>`, `source_profile_id`, `target_profile_ids`, `event_name`, `event_version` |
| `MessageSentEvent` | `author`, `sender_profile_id`, `conversation_id`, `conversation_name`, `conversation_type`, `message_body`, `message_id`, `message_type`, `message_version`, `muted_profile_ids`, `target_profile_ids`, `timestamp`, `event_name`, `event_version`, `trace_parent` |

> Full field types live in `luminary::events`. Every typed event derives `Serialize`/`Deserialize`, so events can be logged or persisted as JSON directly. Unrecognised frames never cause a parse failure — they fall back to `MspEvent::Unknown` instead.

---

## Endpoint Reference

<a id="greetings"></a>
### Greetings — `client.greetings()`

Greetings are accessed through `client.greetings()`. Retrieve the catalogue first, then use a returned greeting type with a target profile ID when sending a greeting.

#### `.get_greeting_definitions() -> Result<Vec<GreetingDefinition>>`
Returns the greeting catalogue available to the authenticated profile.

#### `.send_greeting(greeting_type: &str, profile_id: &str) -> Result<SendGreetingResult>`
Sends the selected greeting to the target profile. Logical failures returned by the service, such as cooldowns or an unavailable type, are surfaced as `MspError::Api`.

<br>

**`GreetingDefinition` fields**

Each `GreetingDefinition` describes a greeting type and its eligibility, cost, cooldown, XP, and reward data.

| Field | Type |
| :--- | :--- |
| `greeting_type` | `String` |
| `hard_cost` | `Option<i64>` |
| `interval_vip` | `Option<i64>` |
| `next_greeting_seconds_remaining` | `Option<i64>` |
| `greeting_max_xp_level_threshold` | `Option<i64>` |
| `greeting_max_xp` | `Option<i64>` |
| `receiver_min_level` | `Option<i64>` |
| `sender_xp_formula` / `receiver_xp_formula` | `Option<String>` |
| `sender_soft_reward` / `receiver_soft_reward` | `Option<i64>` |
| `seasonal_currency` | `Option<String>` |
| `sender_seasonal_reward` / `receiver_seasonal_reward` | `Option<Value>` |
| `sender_currency_rewards` / `receiver_currency_rewards` | `Option<Vec<CurrencyReward>>` |

> `greeting_type` is the identifier used in `.send_greeting(...)`. The `*_xp_formula` fields hold each participant's XP-calculation formula; the `*_reward` and `*_currency_rewards` fields hold each participant's soft, seasonal, and currency rewards.

<a id="attributes"></a>
### Attributes — `client.attributes()`

Attributes represent profile data such as avatar metadata, mood, gender, and free-form `additionalData`. Attribute mutations are performed as a full **GET → modify → PUT** operation on the authenticated profile. Avoid concurrent mutations for the same profile, as a complete replacement can overwrite a parallel update.

#### `.get(profile_id: Option<&str>) -> Result<ProfileAttributes>`
Pass `None` for the authenticated profile, or `Some(profile_id)` for another profile.

#### `.update_additional_data_key(key: &str, value: impl Into<Value>) -> Result<ProfileAttributes>`
Updates one `additionalData` entry on the authenticated profile. Use this for keys without a dedicated helper.

#### `.set_mood(mood: &str) -> Result<ProfileAttributes>`
Sets the authenticated profile's `Mood` value.

#### `.gender_swap() -> Result<ProfileAttributes>`
Toggles the authenticated profile's `Gender` between `"Boy"` and `"Girl"`. Returns `MspError::Api` when the current value is absent or unsupported.

#### `.update_wayd_id(wayd_id: &str) -> Result<ProfileAttributes>`
Sets the authenticated profile's `WAYD` (*What Are You Doing*) status identifier.

<a id="reservations"></a>
### Reservations — `client.reservations()`

A reservation is how the game finds or creates a multiplayer room instance. Chatrooms and the Star Quiz minigame are the two room types this client can reserve.

#### `.chatroom(level: &str, version: &str) -> Result<RoomReservation>`
Reserves a chatroom instance at the given level and version.

#### `.quiz() -> Result<RoomReservation>`
Reserves a Star Quiz room instance. Internally requests asset version `"624"` — the version the game currently ships. There is no discovery mechanism for this; it's simply what the client sends.

<br>

**`RoomReservation` fields**

| Field | Type | Description |
| :--- | :--- | :--- |
| `host_url` | `String` | Base URL of the reserved room instance. |
| `room_id` | `String` | Identifier of the reserved room. |
| `socket_url` | `String` | Ready-to-connect WebSocket URL, built from `host_url` plus the room kind's socket path and Engine.IO version. |

<a id="star-quiz-automation"></a>
### Star Quiz Automation

Two entry points build on `.quiz()` to connect to and play a quiz room autonomously.

#### `.play_star_quiz(success_rate: f64, send_to_chat: bool) -> Result<UnboundedReceiver<QuizEvent>>`
Reserves a quiz room, connects to it, and plays it autonomously — answering questions at the given success rate (`0.0`–`1.0`) and optionally posting the correct answer in chat. Returns a channel of `QuizEvent`s so the caller can observe progress (question shown, answer submitted, round ended, …) without blocking. For finer control, use `.play_star_quiz_ex(...)` instead.

#### `.play_star_quiz_ex(config: QuizConfig) -> Result<UnboundedReceiver<QuizEvent>>`
Same as `.play_star_quiz`, but with full control over behaviour via `QuizConfig` — custom answer-cache path, back-off tuning, whether to keep reconnecting forever, and more.

The quiz session runs on a spawned background task; this method returns as soon as the room is reserved and the answer key and translations are loaded, without waiting for the quiz to finish.

> **Important:** if the automation detects the daily reward cap has been hit, it currently terminates the **entire process** via `std::process::exit` (after emitting a final `QuizEvent::DailyLimitReached` on the returned channel) — not just this background task. This is a deliberate but blunt design choice inherited from the original implementation; worth knowing if you're embedding this client inside a larger long-running application.

<br>

**`QuizConfig` — builder methods**

Tuning knobs for the quiz automation, built with a chainable builder (same pattern as `MspClientBuilder`).

| Method | Default | Description |
| :--- | :--- | :--- |
| `.success_rate(f64)` | `1.0` | Clamped to `0.0`–`1.0`. Chance of submitting the correct answer. |
| `.send_to_chat(bool)` | `false` | Whether to also post the correct answer in chat. |
| `.answer_submit_delay_ms(u64)` | `500` | Delay before submitting an answer in the quiz UI. |
| `.chat_answer_delay_ms(u64)` | `1500` | Delay before posting the answer in chat, when enabled. |
| `.play_forever(bool)` | `true` | Whether the supervisor keeps reconnecting and playing indefinitely. |
| `.reconnect_extra_delay_ms(u64)` | `0` | Extra delay added on top of the back-off before reconnecting. |
| `.initial_backoff_secs(u64)` | `2` | Starting back-off duration after a disconnect. |
| `.max_backoff_secs(u64)` | `60` | Upper bound the back-off duration grows to. |
| `.jitter_max_ms(u64)` | `500` | Maximum random jitter added to back-off timing. |
| `.handshake_timeout_secs(u64)` | `20` | Timeout for the initial WebSocket handshake. |
| `.read_timeout_secs(u64)` | `45` | Timeout for reading from the WebSocket once connected. |
| `.watchdog_interval_secs(u64)` | `5` | How often the supervisor checks session health. |
| `.custom_questions_path(Option<impl Into<PathBuf>>)` | `None` | Optional path to a custom question/answer cache file. |
| `.automatically_learn(bool)` | `true` | Whether newly-seen questions are learned and cached automatically. |

<br>

**`QuizStats` / `QuizStatsSnapshot`**

`QuizStats` is the internal, atomic counter set the supervisor updates as it runs. Since its atomics aren't `Clone`, call `.snapshot()` to get a plain-data `QuizStatsSnapshot` for display or serialization.

| Field | Meaning |
| :--- | :--- |
| `sessions_completed` | Number of quiz sessions that finished normally. |
| `sessions_error` | Number of quiz sessions that ended in an error. |
| `total_reconnects` | Total reconnect attempts made by the supervisor. |
| `questions_seen` | Total questions observed across all sessions. |
| `answers_submitted` | Total answers submitted. |
| `correct_answers` | Total answers submitted that were correct. |

<a id="messaging"></a>
### Messaging — `client.messaging()`

Methods for finding or creating conversations, sending messages, and reading history.

#### `.find_conversation(other_profile_id: &str) -> Result<Option<Conversation>>`
Looks up an existing one-to-one conversation with the given profile. Returns `None` when no conversation exists yet, on a `404`, or when the response body is empty/null.

#### `.create_conversation(other_profile_id: &str) -> Result<Conversation>`
Creates a new one-to-one conversation with the given profile.

#### `.get_or_create_conversation(other_profile_id: &str) -> Result<Conversation>`
Convenience wrapper: returns the existing conversation if `.find_conversation(...)` finds one, otherwise creates one.

#### `.mark_conversation_as_read(conversation_id: &str) -> Result<ConversationEntry>`
Marks a conversation as read for the authenticated profile — resets its unread count and unmutes it.

#### `.send_message(conversation_id: &str, body: &str) -> Result<MessageReceipt>`
Sends a chat message into the given conversation.

#### `.get_conversations(page: u32, page_size: u32) -> Result<ConversationPage>`
Returns a page of the authenticated profile's conversations, along with the IDs of the ones with unread messages. Returns an empty page on a `404` or an empty/null response body.

#### `.get_chat_history(conversation_id: &str, page_size: u32) -> Result<Vec<ChatMessage>>`
Returns the message history for a conversation.

---

<a id="core-features"></a>
## Core Features

- **Browser emulation:** Keeps modern desktop browser profiles and platforms consistent across a session.
- **Background management:** Handles token rotation and connection recovery after login.
- **Proxy support:** Supports proxy configuration and strict proxy enforcement.

---

<a id="disclaimer"></a>
## Disclaimer

**Luminary** is an independent, unofficial developer tool created for educational and experimental use. It is not affiliated with or endorsed by MovieStarPlanet.
