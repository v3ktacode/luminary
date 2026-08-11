use std::net::Ipv6Addr;
use std::sync::Arc;
use std::time::Duration;

use wreq::header::{HeaderMap, HeaderValue};
use wreq::Proxy;
use wreq_util::{Emulation, Platform, Profile};

use crate::config::MspConfig;
use crate::errors::{MspError, Result};
use crate::models::MspSession;
use crate::state::SessionState;
use super::cookies::PersistentJar;
use super::stealth::StealthConfig;
use super::MspClient;
use rand::seq::SliceRandom;


pub struct MspClientBuilder {
    config:                  MspConfig,
    device_id:               Option<String>,
    profile:                 Profile,
    platform:                Platform,
    randomize_platform:      bool,
    randomize_profile_brand: Option<BrowserBrand>,
    proxy_url:               Option<String>,
    enforce_proxy:           bool,
    timeout:                 Duration,
    connect_timeout:         Duration,
    locale:                  Option<String>,
    stealth:                 StealthConfig,
    restore_state:           Option<SessionState>,
    presence:                bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserBrand { Chrome, Firefox, Any }

impl Default for MspClientBuilder {
    fn default() -> Self {
        let config = MspConfig::default();
        let (timeout, connect_timeout) = (config.request_timeout(), config.connect_timeout());
        Self {
            timeout, 
            connect_timeout, 
            config,
            device_id: None, 
            locale: None, 
            proxy_url: None, 
            restore_state: None,
            profile: Profile::Chrome137,
            platform: Platform::Windows,
            randomize_platform: false, 
            randomize_profile_brand: None,
            enforce_proxy: false, 
            stealth: StealthConfig::default(),
            presence: true,
        }
    }
}

impl MspClientBuilder {
    pub fn config(mut self, config: MspConfig) -> Self {
        self.timeout         = config.request_timeout();
        self.connect_timeout = config.connect_timeout();
        self.config = config;
        self
    }

    pub fn device_id(mut self, id: impl Into<String>) -> Self {
        self.device_id = Some(id.into()); self
    }

    pub fn profile(mut self, profile: Profile) -> Self {
        self.profile = profile;
        self.randomize_profile_brand = None;
        self
    }

    pub fn platform(mut self, platform: Platform) -> Self {
        self.platform = platform;
        self.randomize_platform = false;
        self
    }

    pub fn random_platform(mut self) -> Self { self.randomize_platform = true; self }

    pub fn random_profile(mut self, brand: BrowserBrand) -> Self {
        self.randomize_profile_brand = Some(brand); self
    }

    pub fn proxy(mut self, proxy_url: impl Into<String>) -> Self {
        self.proxy_url = Some(proxy_url.into()); self
    }

    pub fn enforce_proxy(mut self, enforce: bool) -> Self {
        self.enforce_proxy = enforce; self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self { self.timeout = timeout; self }

    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout; self
    }

    pub fn locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = Some(locale.into()); self
    }

    pub fn stealth(mut self, min: Duration, max: Duration) -> Self {
        if min > max {
            tracing::warn!(?min, ?max,
                "stealth(): min > max, StealthConfig will swap them automatically");
        }
        self.stealth = StealthConfig::enabled(min, max);
        self
    }

    pub fn from_state(mut self, state: SessionState) -> Self {
        self.restore_state = Some(state); self
    }

    /// Enable or disable Presence WebSocket connection.
    ///
    /// When `false`, the client will skip connecting to the presence server
    /// after login. Defaults to `true`.
    pub fn presence(mut self, enabled: bool) -> Self {
        self.presence = enabled;
        self
    }

    pub fn build(self) -> Result<MspClient> { run_async(self.build_async()) }

    /// Runs an async future to completion from a sync context.
    ///
    /// Note: if a `current_thread` Tokio runtime is already running on this
    /// thread (as opposed to no runtime, or a `multi_thread` runtime),
    /// `block_in_place` below will panic. Callers embedding `build()` inside
    /// a `#[tokio::main(flavor = "current_thread")]` app should call
    /// `build_async()` directly instead.
    pub async fn build_async(self) -> Result<MspClient> {
        let presence = self.presence;

        if let Some(state) = self.restore_state {
            let client = Self::build_from_state(
                state, self.config, self.timeout, self.connect_timeout,
                self.locale, self.stealth,
            ).await?;
            client.set_presence(presence);
            return Ok(client);
        }

        validate_proxy_config(self.enforce_proxy, &self.proxy_url)?;

        let mut rng = rand::thread_rng();

        let platform = if self.randomize_platform {
            *[Platform::Windows, Platform::MacOS, Platform::Linux]
                .choose(&mut rng).unwrap_or(&Platform::Windows)
        } else {
            self.platform
        };

        let profile = match self.randomize_profile_brand {
            Some(BrowserBrand::Chrome) => *[
                Profile::Chrome133, Profile::Chrome135,
                Profile::Chrome136, Profile::Chrome137,
            ].choose(&mut rng).unwrap_or(&Profile::Chrome137),

            Some(BrowserBrand::Firefox) => *[
                Profile::Firefox133, Profile::Firefox136, Profile::Firefox139,
            ].choose(&mut rng).unwrap_or(&Profile::Firefox139),

            Some(BrowserBrand::Any) => *[
                Profile::Chrome136, Profile::Chrome137, Profile::Firefox139,
            ].choose(&mut rng).unwrap_or(&Profile::Chrome137),

            None => self.profile,
        };

        let platform  = coherent_platform(profile, platform);
        let device_id = self.device_id.unwrap_or_else(|| {
            uuid::Uuid::new_v4().to_string().replace('-', "").to_uppercase()
        });
        let accept_language = self.locale.unwrap_or_else(default_locale);
        let jar = Arc::new(PersistentJar::new());

        let http = build_http_client(
            profile, platform, &self.proxy_url, self.enforce_proxy,
            self.timeout, self.connect_timeout, jar.clone(),
            &accept_language, &self.config.referer,
        )?;

        let client = MspClient::from_parts(
            http, device_id, profile, platform,
            self.proxy_url, self.enforce_proxy, jar, self.stealth, self.config,
        );

        client.set_presence(presence);
        Ok(client)
    }

    async fn build_from_state(
        state: SessionState, config: MspConfig,
        timeout: Duration, connect_timeout: Duration,
        locale: Option<String>, stealth: StealthConfig,
    ) -> Result<MspClient> {
        if !state.is_schema_compatible() {
            return Err(MspError::state(format!(
                "SessionState schema version {} is not compatible with the \
                 current library (v{})",
                state.schema_version, SessionState::SCHEMA_VERSION,
            )));
        }

        validate_proxy_config(state.enforce_proxy, &state.proxy_url)?;

        let profile  = parse_profile(&state.browser_profile);
        let platform = coherent_platform(profile, parse_platform(&state.platform));
        let accept_language = locale
            .unwrap_or_else(|| locale_for_region(&state.region).to_string());
        let jar = Arc::new(PersistentJar::from_state(&state.cookies));

        let http = build_http_client(
            profile, platform, &state.proxy_url, state.enforce_proxy,
            timeout, connect_timeout, jar.clone(),
            &accept_language, &config.referer,
        )?;

        let session = MspSession {
            access_token:            state.access_token,
            refresh_token:           state.refresh_token,
            profile_id:              state.profile_id,
            sub_id:                  state.sub_id,
            device_id:               state.device_id.clone(),
            access_token_expires_at: state.access_token_expires_at,
            region:                  state.region,
        };

        let client = MspClient::from_parts(
            http, state.device_id, profile, platform,
            state.proxy_url, state.enforce_proxy, jar, stealth, config,
        );

        *client.session().inner().write().await = Some(session);
        Ok(client)
    }
}


pub(crate) fn run_async<F, T>(fut: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            // `block_in_place` panics if the current runtime is a
            // `current_thread` runtime rather than `multi_thread`. Catch
            // that here and return a proper error instead of panicking, so
            // callers embedding `build()` inside a
            // `#[tokio::main(flavor = "current_thread")]` app get a clear
            // message pointing them at `build_async()` instead.
            if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::CurrentThread {
                return Err(MspError::state(
                    "MspClientBuilder::build() cannot run inside a current_thread Tokio \
                     runtime (block_in_place is unsupported there). Call \
                     `build_async()` directly instead."
                ));
            }
            tokio::task::block_in_place(|| handle.block_on(fut))
        }
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| MspError::state(format!("failed to start temporary runtime: {e}")))?
            .block_on(fut),
    }
}

fn validate_proxy_config(enforce_proxy: bool, proxy_url: &Option<String>) -> Result<()> {
    if enforce_proxy && proxy_url.is_none() {
        return Err(MspError::proxy(
            "Proxy enforcement is enabled but no proxy URL was provided"
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_http_client(
    profile: Profile, platform: Platform,
    proxy_url: &Option<String>, enforce_proxy: bool,
    timeout: Duration, connect_timeout: Duration,
    jar: Arc<PersistentJar>, accept_language: &str, referer: &str,
) -> Result<wreq::Client> {
    let emulation = Emulation::builder().profile(profile).platform(platform).build();

    let mut builder = wreq::Client::builder()
        .emulation(emulation)
        .cookie_provider(jar)
        .timeout(timeout)
        .connect_timeout(connect_timeout)
        .tcp_nodelay(true)
        .https_only(true)
        .referer(false)
        .default_headers(locale_and_referer_headers(accept_language, referer));

    if let Some(ref url) = proxy_url {
        let normalized = normalize_proxy_string(url)?;
        let proxy = Proxy::all(&normalized)
            .map_err(|e| MspError::proxy(format!("{e}: {normalized}")))?;
        builder = builder.proxy(proxy);
    } else if enforce_proxy {
        return Err(MspError::proxy(
            "Enforce proxy constraint is active, but proxy selection is unassigned"
        ));
    }

    Ok(builder.build().map_err(|e| MspError::proxy(e.to_string()))?)
}

fn locale_and_referer_headers(accept_language: &str, referer: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    match HeaderValue::from_str(accept_language) {
        Ok(v) => { h.insert("accept-language", v); }
        Err(e) => tracing::warn!("Skipping Accept-Language header — invalid value: {e}"),
    }
    match HeaderValue::from_str(referer) {
        Ok(v) => { h.insert(wreq::header::REFERER, v); }
        Err(e) => tracing::warn!("Skipping Referer header — invalid value: {e}"),
    }
    h
}

fn default_locale() -> String { "fr-FR,fr;q=0.9,en-US;q=0.8,en;q=0.7".to_string() }

pub(crate) fn locale_for_region(region: &str) -> &'static str {
    match region.to_ascii_lowercase().as_str() {
        "us" | "en" | "english"  => "en-US,en;q=0.9",
        "gb" | "uk"              => "en-GB,en;q=0.9",
        "fr" | "french"          => "fr-FR,fr;q=0.9,en-US;q=0.8,en;q=0.7",
        "de" | "german"          => "de-DE,de;q=0.9,en-US;q=0.8,en;q=0.7",
        "es" | "spanish"         => "es-ES,es;q=0.9,en-US;q=0.8,en;q=0.7",
        "it" | "italian"         => "it-IT,it;q=0.9,en-US;q=0.8,en;q=0.7",
        "nl" | "dutch"           => "nl-NL,nl;q=0.9,en;q=0.8",
        "da" | "danish"          => "da-DK,da;q=0.9,en;q=0.8",
        "sv" | "swedish"         => "sv-SE,sv;q=0.9,en;q=0.8",
        "no" | "norwegian"       => "nb-NO,nb;q=0.9,en;q=0.8",
        "fi" | "finnish"         => "fi-FI,fi;q=0.9,en;q=0.8",
        "pl" | "polish"          => "pl-PL,pl;q=0.9,en;q=0.8",
        "tr" | "turkish"         => "tr-TR,tr;q=0.9,en;q=0.8",
        _                        => "fr-FR,fr;q=0.9,en-US;q=0.8,en;q=0.7",
    }
}

// NOTE: this matches on the `Debug` output of `Profile` because `wreq_util`
// doesn't expose a browser-family accessor. It's a bit fragile — if
// `wreq_util` renames its Safari variants this silently stops matching —
// but there's no stable alternative available from the enum today.
fn coherent_platform(profile: Profile, requested: Platform) -> Platform {
    let name = format!("{profile:?}");
    if name.starts_with("Safari") && !(name.contains("Ios") || name.contains("IPad")) {
        if requested != Platform::MacOS {
            tracing::debug!(
                profile = %name, requested = ?requested,
                "Overriding requested platform to MacOS for desktop Safari profile coherence"
            );
        }
        Platform::MacOS
    } else {
        requested
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownVariant(pub String);

impl std::fmt::Display for UnknownVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown variant: {}", self.0)
    }
}
impl std::error::Error for UnknownVariant {}

macro_rules! impl_from_str_wrapper {
    ($Wrapper:ident, $Inner:ty, $($s:literal => $v:expr),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $Wrapper(pub $Inner);

        impl std::str::FromStr for $Wrapper {
            type Err = UnknownVariant;
            fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
                match s {
                    $($s => Ok($Wrapper($v)),)+
                    other => Err(UnknownVariant(other.to_string())),
                }
            }
        }
    };
}

impl_from_str_wrapper!(ProfileWrapper, Profile,
    "Chrome118"  => Profile::Chrome118,
    "Chrome124"  => Profile::Chrome124,
    "Chrome128"  => Profile::Chrome128,
    "Chrome131"  => Profile::Chrome131,
    "Chrome133"  => Profile::Chrome133,
    "Chrome135"  => Profile::Chrome135,
    "Chrome136"  => Profile::Chrome136,
    "Chrome137"  => Profile::Chrome137,
    "Firefox117" => Profile::Firefox117,
    "Firefox128" => Profile::Firefox128,
    "Firefox133" => Profile::Firefox133,
    "Firefox136" => Profile::Firefox136,
    "Firefox139" => Profile::Firefox139,
);

impl_from_str_wrapper!(PlatformWrapper, Platform,
    "Windows" => Platform::Windows,
    "MacOS"   => Platform::MacOS,
    "Linux"   => Platform::Linux,
);

pub(crate) fn parse_profile(s: &str) -> Profile {
    s.parse::<ProfileWrapper>().map(|w| w.0).unwrap_or_else(|_| {
        tracing::warn!(profile = %s, fallback = "Chrome137", "Unknown browser profile in SessionState");
        Profile::Chrome137
    })
}

pub(crate) fn parse_platform(s: &str) -> Platform {
    s.parse::<PlatformWrapper>().map(|w| w.0).unwrap_or_else(|_| {
        tracing::warn!(platform = %s, fallback = "Windows", "Unknown platform in SessionState");
        Platform::Windows
    })
}


fn extract_scheme(raw: &str) -> (&'static str, &str) {
    for (prefix, scheme) in [
        ("socks5h://", "socks5h"),
        ("socks5://",  "socks5"),
        ("https://",   "https"),
        ("http://",    "http"),
    ] {
        if let Some(rest) = raw.strip_prefix(prefix) { return (scheme, rest); }
    }
    ("http", raw)
}

/// Normalizes a user-supplied proxy string into a `scheme://[user:pass@]host:port`
/// URL that `wreq::Proxy::all` can consume.
///
/// Two input shapes are accepted:
/// - Already-URL-shaped: `[scheme://]user:pass@host:port` (returned as-is, after
///   validating that a port is present).
/// - IP:PORT-shaped, optionally with trailing credentials:
///   `[scheme://]host:port[:user[:pass]]`, e.g. `1.2.3.4:8080:user:pass` or
///   `socks5h://[2001:db8::1]:1080:user:pass`. Passwords may themselves contain
///   colons — everything after the user segment is taken verbatim as the password.
///
/// Credentials are percent-encoded into the resulting userinfo component so that
/// characters like `@` or `:` inside a password can't be misparsed as URL
/// delimiters.
fn normalize_proxy_string(raw: &str) -> Result<String> {
    let (scheme, rest) = extract_scheme(raw);

    // Already in `user:pass@host:port` form — just validate and pass through.
    if let Some(at_idx) = rest.rfind('@') {
        let hostport = &rest[at_idx + 1..];
        validate_hostport(hostport, raw)?;
        return Ok(format!("{scheme}://{rest}"));
    }

    // IPv6 literal host: `[host]:port[:user[:pass]]`.
    if let Some(after_bracket) = rest.strip_prefix('[') {
        let end = after_bracket.find(']')
            .ok_or_else(|| MspError::proxy(format!("Unterminated IPv6 literal in: {raw}")))?;

        let ipv6 = &after_bracket[..end];
        ipv6.parse::<Ipv6Addr>()
            .map_err(|_| MspError::proxy(format!("Invalid IPv6 address in: {raw}")))?;

        let host = &rest[..=end + 1]; // includes surrounding brackets
        let remainder = rest[end + 2..].strip_prefix(':')
            .ok_or_else(|| MspError::proxy(format!("Missing port in: {raw}")))?;

        let mut parts = remainder.splitn(3, ':');
        let port = parts.next().unwrap_or_default();
        validate_port(port, raw)?;
        let user = parts.next();
        let pass = parts.next();

        return Ok(build_proxy_url(scheme, host, port, user, pass));
    }

    // Plain host: `host:port[:user[:pass]]`. `splitn(4, ..)` keeps any colons
    // inside the password intact instead of splitting on them.
    let mut parts = rest.splitn(4, ':');
    let host = parts.next().filter(|s| !s.is_empty())
        .ok_or_else(|| MspError::proxy(format!("Invalid proxy format: {raw}")))?;
    let port = parts.next()
        .ok_or_else(|| MspError::proxy(format!("Missing port in: {raw}")))?;
    validate_port(port, raw)?;
    let user = parts.next();
    let pass = parts.next();

    Ok(build_proxy_url(scheme, host, port, user, pass))
}

/// Validates that a bare `host:port` (or `[ipv6]:port`) string has a
/// well-formed, non-zero port. Used for the already-has-credentials branch
/// of `normalize_proxy_string`, where the host/port themselves don't need
/// to be rebuilt — only checked.
fn validate_hostport(hostport: &str, raw: &str) -> Result<()> {
    if let Some(after_bracket) = hostport.strip_prefix('[') {
        let end = after_bracket.find(']')
            .ok_or_else(|| MspError::proxy(format!("Unterminated IPv6 literal in: {raw}")))?;
        after_bracket[..end].parse::<Ipv6Addr>()
            .map_err(|_| MspError::proxy(format!("Invalid IPv6 address in: {raw}")))?;
        let port = after_bracket[end + 1..].strip_prefix(':')
            .ok_or_else(|| MspError::proxy(format!("Missing port in: {raw}")))?;
        return validate_port(port, raw);
    }

    let port = hostport.rfind(':')
        .map(|idx| &hostport[idx + 1..])
        .ok_or_else(|| MspError::proxy(format!("Missing port in: {raw}")))?;
    validate_port(port, raw)
}

/// Assembles `scheme://[user[:pass]@]host:port`, percent-encoding credentials.
fn build_proxy_url(scheme: &str, host: &str, port: &str, user: Option<&str>, pass: Option<&str>) -> String {
    match user {
        Some(user) => {
            let userinfo = match pass {
                Some(pass) => format!("{}:{}", encode_userinfo(user), encode_userinfo(pass)),
                None => encode_userinfo(user),
            };
            format!("{scheme}://{userinfo}@{host}:{port}")
        }
        None => format!("{scheme}://{host}:{port}"),
    }
}

fn validate_port(port: &str, raw: &str) -> Result<()> {
    match port.parse::<u16>() {
        Ok(0) => Err(MspError::proxy(format!("Proxy port cannot be 0 in: {raw}"))),
        Err(_) => Err(MspError::proxy(format!("Invalid proxy port '{port}' in: {raw}"))),
        Ok(_)  => Ok(()),
    }
}

fn encode_userinfo(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}


#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! assert_proxy {
        ($input:expr, $expected:expr) => {
            assert_eq!(normalize_proxy_string($input).unwrap(), $expected);
        };
        (err $input:expr) => {
            assert!(normalize_proxy_string($input).is_err());
        };
    }

    #[test] fn plain_host_port_user_pass() {
        assert_proxy!("1.2.3.4:8080:user:pass", "http://user:pass@1.2.3.4:8080"); }
    #[test] fn password_with_colons() {
        assert_proxy!("1.2.3.4:8080:user:p@ss:w0rd", "http://user:p%40ss%3Aw0rd@1.2.3.4:8080"); }
    #[test] fn preserves_scheme() {
        assert_proxy!("socks5h://1.2.3.4:1080:user:pass", "socks5h://user:pass@1.2.3.4:1080"); }
    #[test] fn leaves_well_formed_url_untouched() {
        assert_proxy!("http://user:pass@1.2.3.4:8080", "http://user:pass@1.2.3.4:8080"); }
    #[test] fn adds_scheme_to_bare_host_port() {
        assert_proxy!("1.2.3.4:8080", "http://1.2.3.4:8080"); }
    #[test] fn ipv6_host_port() {
        assert_proxy!("[::1]:1080", "http://[::1]:1080"); }
    #[test] fn ipv6_with_credentials() {
        assert_proxy!("socks5h://[2001:db8::1]:1080:user:pass",
            "socks5h://user:pass@[2001:db8::1]:1080"); }
    #[test] fn rejects_invalid_port()   { assert_proxy!(err "1.2.3.4:notaport:user:pass"); }
    #[test] fn rejects_garbage()        { assert_proxy!(err "not a proxy at all"); }
    #[test] fn rejects_ipv6_no_port()   { assert_proxy!(err "[::1]"); }
    #[test] fn rejects_bad_ipv6()       { assert_proxy!(err "[not::an::ipv6]:1080"); }
    #[test] fn rejects_port_zero()      { assert_proxy!(err "1.2.3.4:0:user:pass"); }

    #[test] fn profile_wrapper_roundtrip() {
        assert_eq!("Chrome137".parse::<ProfileWrapper>().unwrap().0, Profile::Chrome137);
        assert!("NotAProfile".parse::<ProfileWrapper>().is_err());
    }
    #[test] fn platform_wrapper_roundtrip() {
        assert_eq!("MacOS".parse::<PlatformWrapper>().unwrap().0, Platform::MacOS);
        assert!("NotAPlatform".parse::<PlatformWrapper>().is_err());
    }
}