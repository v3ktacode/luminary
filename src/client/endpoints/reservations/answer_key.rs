// src/client/endpoints/reservations/answer_key.rs
//
// The "answer key" is a map from a question's localization key to the index
// of its correct answer. It's what lets `play_star_quiz` answer questions
// automatically instead of guessing.
//
// Two sources are supported:
//   - A community-maintained answer list fetched from GitHub on startup.
//   - An optional local JSON file (`QuizConfig::custom_questions_path`),
//     which — if present — is used instead of the remote fetch, and is also
//     where newly-learned answers get persisted (see `automatically_learn`
//     in `session.rs`).

use std::collections::HashMap;

use rand::Rng;
use wreq::Client;

use super::quiz_config::QuizConfig;

/// Community-maintained question/answer list, used as the default source
/// when no local cache file is configured (or it doesn't exist yet).
const QUESTIONS_URL: &str =
    "https://raw.githubusercontent.com/v3ktacode/apex-moviestarplanet2/refs/heads/main/questions.json";

/// Picks an answer for a question.
///
/// If the answer key has an entry for `q`, it's used with probability `rate`
/// (clamped to `[0, 1]`); otherwise — or the rest of the time — a random
/// answer from 1 to 3 is returned. This lets `success_rate` simulate
/// imperfect play instead of being suspiciously always-correct.
pub(super) fn pick_answer(q: &str, key: &HashMap<String, u32>, rate: f64) -> u32 {
    let mut rng = rand::thread_rng();
    if let Some(&c) = key.get(q) {
        if rng.gen::<f64>() < rate.clamp(0.0, 1.0) {
            return c;
        }
    }
    rng.gen_range(1u32..=3)
}

async fn fetch_answer_key(
    http: &Client,
) -> std::result::Result<HashMap<String, u32>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(http.get(QUESTIONS_URL).send().await?.json().await?)
}

/// Loads the answer key, preferring the local cache file when configured
/// and present, and falling back to (then seeding) the remote list otherwise.
pub(super) async fn load_answer_key(
    config: &QuizConfig,
    http: &Client,
) -> std::result::Result<HashMap<String, u32>, Box<dyn std::error::Error + Send + Sync>> {
    if let Some(ref path) = config.custom_questions_path {
        if path.exists() {
            let content = tokio::fs::read_to_string(path).await?;
            return Ok(serde_json::from_str(&content)?);
        }
    }

    let map = fetch_answer_key(http).await?;

    // Seed the local cache file with the remote list so future runs don't
    // need network access to bootstrap, and so `automatically_learn` has a
    // file to update.
    if let Some(ref path) = config.custom_questions_path {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, serde_json::to_string_pretty(&map)?).await?;
    }

    Ok(map)
}