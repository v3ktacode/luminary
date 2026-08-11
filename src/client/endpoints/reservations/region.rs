// src/client/endpoints/reservations/region.rs
//
// Small region → locale mappings used when reserving rooms and fetching
// quiz translations. These are two separate tables because the two target
// systems use different locale formats: room reservations expect a
// BCP-47-ish culture tag (`fr-FR`), while the quiz translation files are
// keyed by an underscore-separated language code (`fr_FR`).

/// Maps a two-letter region code to the culture tag expected by the room
/// reservation payload (`Parameters.Culture`).
pub(super) fn region_to_culture(region: &str) -> &'static str {
    match region.to_uppercase().as_str() {
        "FR" => "fr-FR",
        "TR" => "tr-TR",
        "PL" => "pl-PL",
        "DE" => "de-DE",
        "IT" => "it-IT",
        "US" => "en-US",
        "GB" => "en-GB",
        "CA" => "en-CA",
        "ES" => "es-ES",
        "DK" => "da-DK",
        "NL" => "nl-NL",
        "NO" => "nb-NO",
        "FI" => "fi-FI",
        "SE" => "sv-SE",
        _ => "en-US",
    }
}

/// Maps a two-letter region code to the language code used in the quiz
/// translation file path (`.../quiz/{lang}/localization_data.txt`).
pub(super) fn region_to_lang_code(region: &str) -> &'static str {
    match region.to_uppercase().as_str() {
        "FR" => "fr_FR",
        "TR" => "tr_TR",
        "PL" => "pl_PL",
        "DE" => "de_DE",
        "US" => "en_US",
        "GB" => "en-GB",
        "ES" => "es_ES",
        "DK" => "da_DK",
        "NL" => "nl_NL",
        "CA" => "en-CA",
        "NO" => "nb_NO",
        "FI" => "fi_FI",
        "SE" => "sv_SE",
        _ => "en_US",
    }
}