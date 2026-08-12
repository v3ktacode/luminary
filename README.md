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

## Authentication

`client.auth().login(username, password, region)` runs the login flow, stores the resulting session, and starts the configured background services. The `region` value is case-insensitive and normalised internally.

| Outcome | Value | Meaning |
| :--- | :--- | :--- |
| Success | `MspSession` | A profile-scoped session containing tokens, the profile ID, device ID, expiry, and region. |
| Invalid credentials | `MspError::InvalidCredentials { username, region }` | The supplied account credentials were rejected. |
| Banned account | `MspError::AccountBanned { username, region, reason }` | The account is permanently restricted; `reason` contains the server-provided context. |
| Other failure | `MspError` | Covers other authentication, transport, or response-processing failures. |

```rust
match client.auth().login("username", "password", "FR").await {
    Ok(session) => {
        println!("Successfully authenticated profile ID: {}", session.profile_id);
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

---

## Endpoint Reference

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

---

## Core Features

- **Browser emulation:** Keeps modern desktop browser profiles and platforms consistent across a session.
- **Background management:** Handles token rotation and connection recovery after login.
- **Proxy support:** Supports proxy configuration and strict proxy enforcement.

---

## Disclaimer

**Luminary** is an independent, unofficial developer tool created for educational and experimental use. It is not affiliated with or endorsed by MovieStarPlanet.
