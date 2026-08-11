use std::collections::HashMap;
use std::sync::Mutex;

use wreq::cookie::{Cookies, CookieStore, Jar};
use wreq::header::HeaderValue;
use wreq::{Uri, Version};

use crate::state::SerializableCookie;

const BASE_URI: &str = "https://moviestarplanet2.com";

#[inline]
fn cookie_key(domain: &str, path: &str, name: &str) -> String {
    format!("{domain}{path}:{name}")
}

#[derive(Default)]
pub struct PersistentJar {
    inner:  Jar,
    shadow: Mutex<HashMap<String, SerializableCookie>>,
}

impl PersistentJar {
    pub fn new() -> Self { Self::default() }

    pub fn from_state(cookies: &HashMap<String, SerializableCookie>) -> Self {
        let jar = Self::new();
        let valid: Vec<&SerializableCookie> = cookies.values().filter(|c| !c.is_expired()).collect();

        if let Ok(uri) = BASE_URI.parse::<Uri>() {
            let headers: Vec<HeaderValue> = valid.iter().filter_map(|c| to_set_cookie_header(c)).collect();
            if !headers.is_empty() { jar.inner.set_cookies(&mut headers.iter(), &uri); }
        }

        if let Ok(mut shadow) = jar.shadow.lock() {
            for cookie in &valid {
                shadow.insert(cookie_key(&cookie.domain, &cookie.path, &cookie.name), (*cookie).clone());
            }
        }
        jar
    }

    pub fn export(&self) -> HashMap<String, SerializableCookie> {
        self.shadow.lock()
            .map(|g| g.iter().filter(|(_, c)| !c.is_expired()).map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }
}

impl CookieStore for PersistentJar {
    fn set_cookies(&self, cookie_headers: &mut dyn Iterator<Item = &HeaderValue>, uri: &Uri) {
        let collected: Vec<HeaderValue> = cookie_headers.cloned().collect();

        if let Ok(mut shadow) = self.shadow.lock() {
            for hv in &collected {
                if let Some(cookie) = parse_set_cookie(hv, uri) {
                    let key = cookie_key(&cookie.domain, &cookie.path, &cookie.name);
                    if cookie.value.is_empty() || cookie.is_expired() {
                        shadow.remove(&key);
                    } else {
                        shadow.insert(key, cookie);
                    }
                }
            }
        }

        self.inner.set_cookies(&mut collected.iter(), uri);
    }

    fn cookies(&self, uri: &Uri, version: Version) -> Cookies {
        self.inner.cookies(uri, version)
    }
}


fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn parse_set_cookie(hv: &HeaderValue, uri: &Uri) -> Option<SerializableCookie> {
    let raw = hv.to_str().ok()?;
    let mut it = raw.split(';');

    let (name, value) = it.next()?.split_once('=')?;
    let name = name.trim().to_string();
    if name.is_empty() { return None; }
    let value = value.trim().to_string();

    let mut domain    = uri.host().unwrap_or_default().to_string();
    let mut path      = "/".to_string();
    let mut secure    = false;
    let mut http_only = false;
    let mut same_site  = None::<String>;
    let mut expires_at = None::<i64>;
    let mut max_age_at = None::<i64>;

    for attr in it {
        let attr = attr.trim();
        let (key, val) = match attr.split_once('=') {
            Some((k, v)) => (k.trim().to_ascii_lowercase(), Some(v.trim().to_string())),
            None         => (attr.to_ascii_lowercase(), None),
        };

        match key.as_str() {
            "domain"   => { if let Some(v) = val { domain = v.trim_start_matches('.').to_string(); } }
            "path"     => { if let Some(v) = val { if !v.is_empty() { path = v; } } }
            "secure"   => secure    = true,
            "httponly"  => http_only = true,
            "samesite" => same_site  = val.and_then(|v| normalize_same_site(&v)),
            "max-age"  => {
                if let Some(Ok(secs)) = val.map(|v| v.parse::<i64>()) {
                    max_age_at = Some(if secs <= 0 { 0 } else { now_secs() + secs });
                }
            }
            "expires"  => expires_at = val.and_then(|v| parse_http_date(&v)),
            _ => {}
        }
    }

    Some(SerializableCookie {
        name, value, domain, path, secure, http_only, same_site,
        expires_at: max_age_at.or(expires_at),
    })
}

fn to_set_cookie_header(c: &SerializableCookie) -> Option<HeaderValue> {
    let mut s = format!("{}={}", c.name, c.value);
    if !c.domain.is_empty() { s.push_str(&format!("; Domain={}", c.domain)); }
    if !c.path.is_empty()   { s.push_str(&format!("; Path={}", c.path));     }
    if c.secure    { s.push_str("; Secure");   }
    if c.http_only { s.push_str("; HttpOnly"); }
    if let Some(ss) = &c.same_site { s.push_str(&format!("; SameSite={ss}")); }
    HeaderValue::from_str(&s).ok()
}

fn normalize_same_site(v: &str) -> Option<String> {
    match v.trim().to_ascii_lowercase().as_str() {
        "strict" => Some("Strict".into()),
        "lax"    => Some("Lax".into()),
        "none"   => Some("None".into()),
        _        => None,
    }
}

fn parse_http_date(s: &str) -> Option<i64> {
    Some(httpdate::parse_http_date(s).ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0))
}