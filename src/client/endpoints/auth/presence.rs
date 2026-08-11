// src/client/endpoints/auth/presence.rs
//
// The Presence WebSocket connection — connection management, handshake,
// heartbeat loop, and frame handling.
//
// The entry point for the rest of the auth module is `spawn_presence`, which
// starts a supervised task that reconnects automatically on failure.  The
// only clean exit is when the server sends a FORCE_CLOSE frame with
// `shouldReconnect: false`.

use std::sync::Arc;
use std::time::Instant;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use wreq::ws::message::Message;
use wreq::Client;

use super::{
    timeouts, AuthEndpoint, MspConfig, MspSession, PresenceSlot, WsError, GAME_ID, ORIGIN,
};
use crate::{
    event_bus::EventBus,
    events::parse_frame,
};

// ---------------------------------------------------------------------------
// Presence server message type identifiers
// ---------------------------------------------------------------------------

// These numeric string identifiers come straight from the presence server's
// Socket.IO protocol.  We only handle the ones that require a response or
// that change the connection lifecycle.

/// The server is telling us the account was logged in from somewhere else.
/// We must acknowledge this (see `ACK_LOGGED_IN_ELSEWHERE`) or the server
/// will eventually force-close the socket.
const MSG_LOGGED_IN_ELSEWHERE: &str = "11";

/// The server wants us to disconnect.  The payload includes a
/// `shouldReconnect` boolean — if false, we stop entirely; if true, we
/// reconnect after a short back-off.
const MSG_FORCE_CLOSE: &str = "12";

/// The acknowledgement we send back when we receive `MSG_LOGGED_IN_ELSEWHERE`.
const MSG_ACK_LOGGED_IN_ELSEWHERE: &str = "44";

// ---------------------------------------------------------------------------
// Outcome type
// ---------------------------------------------------------------------------

/// What the presence loop should do after it exits.
#[derive(Debug)]
enum PresenceOutcome {
    /// Something went wrong or the server closed the connection normally.
    /// The supervisor will wait for the back-off delay and then reconnect.
    Reconnect,
    /// The server explicitly told us not to reconnect
    /// (`shouldReconnect: false` in a FORCE_CLOSE message).
    /// The supervisor will stop entirely.
    Stop,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Spawns a new presence supervisor task for the given session.
///
/// If there's already a presence task running (stored in `slot`), it is
/// aborted before the new one starts so we never have two connections open
/// for the same session at once.
pub(super) async fn spawn_presence(
    endpoint: &AuthEndpoint<'_>,
    session: MspSession,
    region: String,
    slot: PresenceSlot,
) {
    let http   = endpoint.http.clone();
    let bus    = endpoint.event_bus.clone();
    let config = Arc::clone(&endpoint.config);

    let handle = tokio::spawn(async move {
        presence_supervisor(http, session, region, bus, config).await;
    });

    if let Some(stale) = slot.lock().await.replace(handle.abort_handle()) {
        stale.abort();
        tracing::debug!("Aborted stale presence task.");
    }
}

// ---------------------------------------------------------------------------
// Supervisor
// ---------------------------------------------------------------------------

/// Long-running task that keeps the presence WebSocket alive.
///
/// Wraps `run_presence_websocket` in a retry loop with exponential back-off.
/// The only way out is if the server sends a FORCE_CLOSE frame with
/// `shouldReconnect: false`.
async fn presence_supervisor(
    http:    Client,
    session: MspSession,
    region:  String,
    bus:     EventBus,
    config:  Arc<MspConfig>,
) {
    use timeouts::{PRESENCE_BACKOFF_MAX, PRESENCE_BACKOFF_MIN};
    use super::grants::jitter;

    let mut backoff = PRESENCE_BACKOFF_MIN;

    loop {
        match run_presence_websocket(&http, &session, &region, &bus, &config).await {
            Ok(PresenceOutcome::Reconnect) => {
                tracing::warn!("Presence socket closed — reconnecting.");
                backoff = PRESENCE_BACKOFF_MIN; // reset on a clean close
            }
            Ok(PresenceOutcome::Stop) => {
                tracing::info!(
                    "Presence server requested permanent disconnect \
                     (shouldReconnect: false). Stopping supervisor."
                );
                return;
            }
            Err(e) => {
                tracing::error!("Presence socket error: {e:?}. Reconnecting.");
                // Keep the current back-off value; it will double below.
            }
        }

        tokio::time::sleep(backoff + jitter(1_000)).await;
        backoff = (backoff * 2).min(PRESENCE_BACKOFF_MAX);
    }
}

// ---------------------------------------------------------------------------
// WebSocket connection
// ---------------------------------------------------------------------------

/// Connects to the presence WebSocket, completes the handshake, and runs the
/// event loop until the connection closes or an error occurs.
///
/// Returns `Ok(outcome)` for clean exits and `Err(…)` for unexpected failures.
async fn run_presence_websocket(
    http:    &Client,
    session: &MspSession,
    region:  &str,
    bus:     &EventBus,
    config:  &MspConfig,
) -> std::result::Result<PresenceOutcome, WsError> {
    use timeouts::{
        PRESENCE_ENGINE_PING_INTERVAL,
        PRESENCE_PING_INTERVAL,
        PRESENCE_READ_TIMEOUT,
    };

    tracing::info!("Connecting to Presence WebSocket…");

    let (mut write, mut read) = http
        .websocket(config.presence_ws_regional(region))
        .header("origin", ORIGIN)
        .send()
        .await?
        .into_websocket()
        .await?
        .split();

    // -- Handshake -----------------------------------------------------------
    //
    // The server speaks Engine.IO on top of WebSocket.  The exact sequence
    // below was determined by observing the Unity client's traffic:
    //
    //   1. Server sends its hello frame (Engine.IO "open").
    //   2. We send a raw Engine.IO ping ("2").
    //   3. Server responds with the Socket.IO connect frame ("40").
    //   4. We send an initial 500-type heartbeat, then our login frame (10).

    recv_expecting(&mut read, "server hello").await?;
    write.send(Message::text("2")).await?;
    recv_expecting(&mut read, "socket.io connect (40)").await?;

    // Each presence session gets its own UUID so the server can distinguish
    // multiple concurrent connections for the same account.
    let session_uuid = uuid::Uuid::new_v4().to_string();

    write
        .send(engine_frame("500", json!({ "pingId": 1, "lastPingDelay": 819_447 })))
        .await?;

    write
        .send(engine_frame(
            "10",
            json!({
                "username":      session.profile_id,
                "access_token":  session.access_token,
                "applicationId": GAME_ID,
                "country":       region,
                "sessionId":     session_uuid,
                "version":       5,
            }),
        ))
        .await?;

    tracing::debug!("Presence handshake complete.");

    // -- Event loop ----------------------------------------------------------

    let mut ping_interval        = tokio::time::interval(PRESENCE_PING_INTERVAL);
    let mut engine_ping_interval = tokio::time::interval(PRESENCE_ENGINE_PING_INTERVAL);
    engine_ping_interval.tick().await; // consume the immediate first tick

    // `ping_id` and `last_ping_delay` mirror what the Unity client sends.
    // The server echoes the ping_id back in its 501 response, which we
    // forward to the event bus as a `PingResponse` event.
    let mut ping_id:         u64     = 2;
    let mut last_ping_delay: u64     = 824_448;
    let mut last_activity:   Instant = Instant::now();

    loop {
        tokio::select! {
            // Application-level heartbeat (message type "500").
            _ = ping_interval.tick() => {
                if last_activity.elapsed() > PRESENCE_READ_TIMEOUT {
                    return Err(
                        "presence inactivity timeout — connection likely dead".into()
                    );
                }
                write.send(engine_frame("500", json!({
                    "pingId":        ping_id,
                    "lastPingDelay": last_ping_delay,
                }))).await?;
                tracing::debug!(ping_id, "Heartbeat sent.");
                ping_id         += 1;
                last_ping_delay += 5_000;
            }

            // Raw Engine.IO ping ("2") — kept separate from the application
            // heartbeat above because the server tracks them independently.
            _ = engine_ping_interval.tick() => {
                write.send(Message::text("2")).await?;
                tracing::debug!("Engine.IO ping sent.");
            }

            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        last_activity = Instant::now();
                        let text = text.to_string();
                        tracing::debug!(frame = %text, "Frame received.");

                        if let Some(outcome) =
                            handle_frame(&text, &mut write, bus).await?
                        {
                            return Ok(outcome);
                        }
                    }

                    // Binary frames and pongs don't carry application data
                    // but do reset the inactivity timer.
                    Some(Ok(Message::Binary(_) | Message::Ping(_) | Message::Pong(_))) => {
                        last_activity = Instant::now();
                    }

                    Some(Ok(Message::Close(_))) | None => {
                        tracing::warn!("Presence disconnected.");
                        return Ok(PresenceOutcome::Reconnect);
                    }

                    Some(Err(e)) => return Err(Box::new(e)),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Frame helpers
// ---------------------------------------------------------------------------

/// Builds a Socket.IO `42[…]` frame that wraps one of the presence server's
/// application-level messages.
///
/// The server expects this exact layout (observed from the Unity client):
/// ```text
/// 42["<messageType>", "{\"messageType\":\"…\",\"messageContent\":{…}}"]
/// ```
/// The inner JSON is serialised as a *string* inside the outer array, not
/// as a nested object.
fn engine_frame(msg_type: &str, content: Value) -> Message {
    let inner = json!({
        "messageType":    msg_type,
        "messageContent": content,
    })
    .to_string();
    Message::text(format!("42{}", json!([msg_type, inner])))
}

/// Extracts the application-level JSON payload from a Socket.IO `42[…]` frame.
///
/// Returns `None` for frames that don't start with `"42"` or can't be parsed.
/// The second element of the outer array can be either a JSON string (which
/// we parse again) or an inline object — we handle both shapes.
fn extract_payload(text: &str) -> Option<Value> {
    let body = text.strip_prefix("42")?;
    let arr: Value = serde_json::from_str(body).ok()?;
    let items = arr.as_array()?;
    let raw = items.get(1)?;

    if let Some(s) = raw.as_str() {
        serde_json::from_str(s).ok()
    } else {
        Some(raw.clone())
    }
}

/// Waits for the next WebSocket message, timing out after
/// `PRESENCE_HANDSHAKE_TIMEOUT`.
///
/// Used during the initial handshake sequence where we must receive specific
/// frames in a fixed order before entering the normal event loop.
/// `label` is only used in error messages to make debugging easier.
async fn recv_expecting<S, E>(
    read:  &mut S,
    label: &str,
) -> std::result::Result<(), WsError>
where
    S: StreamExt<Item = std::result::Result<Message, E>> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    use timeouts::PRESENCE_HANDSHAKE_TIMEOUT;

    match tokio::time::timeout(PRESENCE_HANDSHAKE_TIMEOUT, read.next()).await {
        Ok(Some(Ok(Message::Text(msg)))) => {
            tracing::debug!("{label}: {msg}");
            Ok(())
        }
        Ok(Some(Ok(_)))  => Ok(()),
        Ok(Some(Err(e))) => Err(Box::new(e)),
        Ok(None)         => Err(format!("connection closed while awaiting {label}").into()),
        Err(_)           => Err(format!("timed out awaiting {label}").into()),
    }
}

/// Processes a single Socket.IO text frame from the presence server.
///
/// Returns `Some(outcome)` when the event loop should exit, `None` to keep
/// running.  Any frame that isn't one of the lifecycle message types is
/// forwarded to the event bus via `parse_frame`.
async fn handle_frame<W>(
    text:  &str,
    write: &mut W,
    bus:   &EventBus,
) -> std::result::Result<Option<PresenceOutcome>, WsError>
where
    W: SinkExt<Message, Error: std::error::Error + Send + Sync + 'static> + Unpin,
{
    // If we can't extract a structured payload, fall back to the raw frame
    // parser and publish whatever it finds to the event bus.
    let Some(payload) = extract_payload(text) else {
        if let Some(event) = parse_frame(text) {
            bus.publish(event);
        }
        return Ok(None);
    };

    let Some(kind) = payload.get("messageType").and_then(Value::as_str) else {
        if let Some(event) = parse_frame(text) {
            bus.publish(event);
        }
        return Ok(None);
    };

    match kind {
        // The account was accessed from another location.  We must ACK
        // this or the server will eventually force-close the socket.
        MSG_LOGGED_IN_ELSEWHERE => {
            tracing::warn!("Account logged in from another location — sending ACK.");
            let ack   = json!({ "messageType": MSG_ACK_LOGGED_IN_ELSEWHERE }).to_string();
            let frame = Message::text(format!(
                "42{}",
                json!([MSG_ACK_LOGGED_IN_ELSEWHERE, ack])
            ));
            let _ = write.send(frame).await;
        }

        // The server wants us to disconnect.
        MSG_FORCE_CLOSE => {
            let content          = payload.get("messageContent");
            let should_reconnect = content
                .and_then(|c| c.get("shouldReconnect"))
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let reason = content
                .and_then(|c| c.get("reason"))
                .and_then(Value::as_i64)
                .unwrap_or(-1);

            tracing::warn!(should_reconnect, reason, "Server sent force-close.");

            return Ok(Some(if should_reconnect {
                PresenceOutcome::Reconnect
            } else {
                PresenceOutcome::Stop
            }));
        }

        // Everything else goes to the event bus.
        _ => {
            if let Some(event) = parse_frame(text) {
                bus.publish(event);
            }
        }
    }

    Ok(None)
}