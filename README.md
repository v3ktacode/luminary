# Luminary

> **Luminary** is a comprehensive, asynchronous API wrapper for **MovieStarPlanet 2**. As the largest and most complete implementation for the platform, it handles everything from session management to network discretion.

---

## Overview

Luminary provides a structured approach to interacting with MovieStarPlanet 2. Instead of standard, easily-flagged requests, it handles the underlying transport and protocol details so your sessions remain stable and low-profile.

| Layer | Focus | Implementation |
| --- | --- | --- |
| **Transport** | TLS Fidelity | Emulates realistic browser handshakes (JA3/JA4) using BoringSSL. |
| **Protocol** | HTTP/2 Framing | Matches official browser settings, window sizes, and header orders. |
| **Application** | Header Hygiene | Keeps request headers and language settings properly aligned (JA4H). |
| **Lifecycle** | Session & Presence | Manages OAuth2 authentication, persistent cookies, and real-time presence. |

---

## Quick Start & Usage

```rust
use luminary::MspClient;
use wreq_util::{Platform, Profile};

let client = MspClient::builder()
    .profile(Profile::Chrome137)
    .platform(Platform::Windows)
    .locale("fr-FR,fr;q=0.9,en-US;q=0.8,en;q=0.7")
    .presence(true) // should connect to presence server or not
    .build()?;
```

### Authentication & Error Handling

When authenticating against the platform, Luminary provides explicit error types to handle invalid credentials or account restrictions safely:

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
    Err(e) => {
        tracing::error!(error = %e, "Authentication failed.");
    }
}
```

---

## Core Features

- **Browser Emulation:** Accurately mimics modern desktop environments to keep connections natural.

- **Background Management:** Handles automated token rotation and connection recovery behind the scenes.

- **Proxy Support:** Includes strict proxy enforcement to protect IP configurations.

---

## Disclaimer

**Luminary** is an independent, unofficial developer tool created for educational and experimental use. It is not affiliated with or endorsed by MovieStarPlanet.
