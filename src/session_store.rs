// src/session_store.rs
use dashmap::DashMap;
use dashmap::mapref::one::Ref;
use msp_rust::MspClient;

pub struct UserSession {
    pub client:   MspClient,
    pub username: String,
    pub country:  String,
    pub profile_id: String,
}

pub struct SessionStore {
    pub(crate) sessions: DashMap<u64, UserSession>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self { sessions: DashMap::new() }
    }

    pub fn insert(&self, user_id: u64, data: UserSession) {
        self.sessions.insert(user_id, data);
    }

    pub fn remove(&self, user_id: u64) -> bool {
        self.sessions.remove(&user_id).is_some()
    }

    pub fn contains(&self, user_id: u64) -> bool {
        self.sessions.contains_key(&user_id)
    }

    pub fn get(&self, user_id: u64) -> Option<Ref<'_, u64, UserSession>> {
        self.sessions.get(&user_id)
    }

    pub fn with<F, R>(&self, user_id: u64, f: F) -> Option<R>
    where
        F: FnOnce(&UserSession) -> R,
    {
        self.sessions.get(&user_id).map(|s| f(&*s))
    }
}