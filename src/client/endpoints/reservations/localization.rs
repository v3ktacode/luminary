// src/client/endpoints/reservations/localization.rs
//
// Quiz question/answer translation. The quiz WebSocket returns questions and
// answers as opaque localization *keys* (e.g. `QUIZ_123_QUESTION`), not
// human-readable text — the actual strings live in a separate translation
// file the client downloads once per language. This module fetches that
// file and fills in the `translated_*` fields on quiz events.

use std::collections::HashMap;

use wreq::Client;

use crate::events::{QuizInitEvent, QuizQuestionShownEvent};

/// Common shape shared by the two quiz event types that carry a question and
/// a set of answers needing translation. Lets `enrich` work generically over
/// both instead of duplicating the same handful of lines twice.
pub(super) trait Enrichable {
    fn question_key(&self) -> &str;
    fn answer_keys(&self) -> &[String];
    fn expected_answer(&self) -> Option<u32>;
    fn set_translated_question(&mut self, v: Option<String>);
    fn set_translated_answers(&mut self, v: Vec<String>);
    fn set_translated_expected_answer(&mut self, v: Option<String>);
}

impl Enrichable for QuizInitEvent {
    fn question_key(&self) -> &str { &self.question }
    fn answer_keys(&self) -> &[String] { &self.answers }
    fn expected_answer(&self) -> Option<u32> { self.expected_answer }
    fn set_translated_question(&mut self, v: Option<String>) { self.translated_question = v; }
    fn set_translated_answers(&mut self, v: Vec<String>) { self.translated_answers = Some(v); }
    fn set_translated_expected_answer(&mut self, v: Option<String>) { self.translated_expected_answer = v; }
}

impl Enrichable for QuizQuestionShownEvent {
    fn question_key(&self) -> &str { &self.question }
    fn answer_keys(&self) -> &[String] { &self.answers }
    fn expected_answer(&self) -> Option<u32> { self.expected_answer }
    fn set_translated_question(&mut self, v: Option<String>) { self.translated_question = v; }
    fn set_translated_answers(&mut self, v: Vec<String>) { self.translated_answers = Some(v); }
    fn set_translated_expected_answer(&mut self, v: Option<String>) { self.translated_expected_answer = v; }
}

/// Fills in the `translated_*` fields on a quiz event using the loaded
/// localization table. Any answer key with no matching translation falls
/// back to the raw key itself (so the caller still gets *something*
/// displayable) — the question and expected answer fall back to `None`
/// instead, since there's no sane placeholder for those.
pub(super) fn enrich<T: Enrichable>(ev: &mut T, loc: &HashMap<String, String>) {
    ev.set_translated_question(loc.get(ev.question_key()).cloned());

    let translated: Vec<String> = ev
        .answer_keys()
        .iter()
        .map(|k| loc.get(k).cloned().unwrap_or_else(|| k.clone()))
        .collect();
    ev.set_translated_answers(translated);

    if let Some(idx) = ev.expected_answer() {
        // Translation keys follow the pattern `{BASE}_ANSWER{n}`, where BASE
        // is the question key with its `_QUESTION` suffix stripped.
        let base = ev.question_key().strip_suffix("_QUESTION").unwrap_or(ev.question_key());
        let key = format!("{base}_ANSWER{idx}");
        ev.set_translated_expected_answer(loc.get(&key).cloned());
    }
}

/// Downloads the quiz translation table for a given language code (e.g.
/// `"fr_FR"`). The file is a plain `KEY=value` text format, one entry per
/// line — not JSON, so it's parsed by hand rather than through serde.
pub(super) async fn fetch_localization(
    http: &Client,
    lang: &str,
) -> std::result::Result<HashMap<String, String>, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!(
        "https://msp2-static.mspcdns.com/translations/multiplayergames/quiz/{lang}/localization_data.txt"
    );
    tracing::info!(%url, "Fetching quiz translations…");

    let bytes = http.get(&url).send().await?.bytes().await?;
    let (text, _, _) = encoding_rs::UTF_8.decode(&bytes);

    let mut map = HashMap::new();
    for line in text.lines() {
        if let Some(pos) = line.find('=') {
            map.insert(line[..pos].trim().to_owned(), line[pos + 1..].trim().to_owned());
        }
    }
    Ok(map)
}