//! `config.toml`: locale, keys, and weight overrides.
//!
//! One file, hand-edited. Everything in it has a working default except the
//! Discogs token, and a missing key makes its source report itself unconfigured
//! rather than failing — the application is useful with an empty config file and
//! more useful with a filled one.

use std::path::Path;

use serde::Deserialize;

use super::weights::Weights;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// ISO 3166-1 alpha-2. Decides which storefront is asked and therefore what
    /// prices come back and what is region-locked.
    pub locale: String,
    /// ISO 4217, for the sources that let you ask.
    pub currency: String,
    /// An address MusicBrainz and Discogs can use to reach whoever is running
    /// this. Both require a contactable User-Agent and block one without it.
    pub contact: String,
    /// Whether Spotify serves lossless here.
    ///
    /// Rolled out to Premium subscribers from September 2025 across 50-plus
    /// markets, so it is true for most people and false for some. Getting it
    /// wrong moves Spotify a tier in either direction, which is why it is asked
    /// rather than assumed.
    pub spotify_lossless: bool,
    pub keys: Keys,
    pub weights: Weights,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keys {
    /// A personal access token. Without it Discogs refuses to search, so
    /// physical pressings drop out of the ranking entirely.
    pub discogs_token: String,
    /// Qobuz publishes no keys; this is read out of their web player and cached.
    /// Set it by hand to skip that.
    pub qobuz_app_id: String,
    /// A Qobuz account token, from a logged-in `play.qobuz.com` session.
    ///
    /// Their catalogue endpoints answer `401 User authentication is required` to
    /// an app id alone — checked against the live API, not assumed. Empty means
    /// Qobuz is skipped, which costs the ranking its second-best tier-A source.
    pub qobuz_user_token: String,
    /// 7digital's API moved behind a commercial partner agreement when Songtradr
    /// acquired them, so there is no self-serve key to get. The slot is here for
    /// anyone who has one; empty means the source is skipped entirely rather
    /// than tried and reported as broken.
    pub sevendigital_key: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            locale: "US".to_string(),
            currency: "USD".to_string(),
            contact: String::new(),
            spotify_lossless: true,
            keys: Keys::default(),
            weights: Weights::default(),
        }
    }
}

/// What went wrong reading the config.
///
/// A typed outcome rather than an exception: a broken config file is an expected
/// state — someone edits TOML by hand and mistypes a key — and the application
/// carries on with defaults while saying so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigProblem {
    /// No file yet. Not a problem, just a first run.
    Absent,
    Unreadable(String),
    Invalid(String),
}

impl std::fmt::Display for ConfigProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigProblem::Absent => write!(f, "no config file yet"),
            ConfigProblem::Unreadable(why) => write!(f, "config.toml could not be read: {why}"),
            ConfigProblem::Invalid(why) => write!(f, "config.toml has an error: {why}"),
        }
    }
}

impl Settings {
    pub fn path_in(config_dir: &Path) -> std::path::PathBuf {
        config_dir.join("config.toml")
    }

    /// Read the config, or explain why the defaults are being used instead.
    pub fn load(config_dir: &Path) -> (Settings, Option<ConfigProblem>) {
        let path = Settings::path_in(config_dir);
        if !path.exists() {
            return (Settings::default(), Some(ConfigProblem::Absent));
        }
        match std::fs::read_to_string(&path) {
            Err(error) => (
                Settings::default(),
                Some(ConfigProblem::Unreadable(error.to_string())),
            ),
            Ok(text) => Settings::parse(&text),
        }
    }

    pub fn parse(text: &str) -> (Settings, Option<ConfigProblem>) {
        match toml::from_str::<Settings>(text) {
            Ok(settings) => (settings, None),
            // Defaults rather than nothing: a mistyped weight should not stop a
            // person looking an album up, it should tell them and carry on.
            Err(error) => (
                Settings::default(),
                Some(ConfigProblem::Invalid(error.message().to_string())),
            ),
        }
    }

    /// The User-Agent every request goes out with.
    ///
    /// MusicBrainz blocks agents with no contact address, so the address is part
    /// of the string when there is one and the string says so when there is not.
    pub fn user_agent(&self) -> String {
        let version = env!("CARGO_PKG_VERSION");
        if self.contact.trim().is_empty() {
            format!("Sleeve/{version} ( https://github.com/mhagrelius/sleeve )")
        } else {
            format!("Sleeve/{version} ( {} )", self.contact.trim())
        }
    }

    /// Whether MusicBrainz and Discogs will accept our requests.
    pub fn has_contact(&self) -> bool {
        !self.contact.trim().is_empty()
    }

    /// The default file, written on first run so there is something to edit.
    pub fn template() -> &'static str {
        include_str!("config.template.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::offer::Vendor;

    #[test]
    fn an_empty_config_is_valid_and_yields_the_defaults() {
        let (settings, problem) = Settings::parse("");
        assert_eq!(problem, None);
        assert_eq!(settings.locale, "US");
        assert_eq!(settings.weights.payout_for(Vendor::Bandcamp), 15);
    }

    #[test]
    fn a_config_sets_locale_keys_and_weight_overrides_together() {
        let (settings, problem) = Settings::parse(
            r#"
            locale = "GB"
            currency = "GBP"
            contact = "someone@example.com"
            spotify_lossless = false

            [keys]
            discogs_token = "abc123"

            [weights]
            lossless = 7
            "#,
        );
        assert_eq!(problem, None);
        assert_eq!(settings.locale, "GB");
        assert_eq!(settings.keys.discogs_token, "abc123");
        assert_eq!(settings.weights.lossless, 7);
        assert!(!settings.spotify_lossless);
        // Untouched weights keep their defaults.
        assert_eq!(settings.weights.dr_high, 8);
    }

    #[test]
    fn a_broken_config_falls_back_to_defaults_and_says_what_is_wrong() {
        // A mistyped weight must not stop someone looking an album up.
        let (settings, problem) = Settings::parse("locale = ");
        assert_eq!(settings.locale, "US");
        assert!(matches!(problem, Some(ConfigProblem::Invalid(_))));
    }

    #[test]
    fn an_unknown_key_is_reported_rather_than_silently_ignored() {
        // Someone who types `lossles = 7` should be told, not left wondering why
        // their override did nothing.
        let (_, problem) = Settings::parse("[weights]\nlossles = 7");
        assert!(matches!(problem, Some(ConfigProblem::Invalid(_))));
    }

    #[test]
    fn the_user_agent_carries_a_contact_when_there_is_one() {
        let settings = Settings {
            contact: "matthew@hagreli.us".into(),
            ..Settings::default()
        };
        let agent = settings.user_agent();
        assert!(agent.starts_with("Sleeve/"));
        assert!(agent.contains("matthew@hagreli.us"));
        assert!(settings.has_contact());
    }

    #[test]
    fn a_missing_contact_still_produces_a_reachable_user_agent() {
        // MusicBrainz wants a way to get hold of whoever is running this. A
        // project URL is the fallback; an agent with neither gets blocked.
        let settings = Settings::default();
        assert!(settings.user_agent().contains("github.com"));
        assert!(!settings.has_contact());
    }

    #[test]
    fn a_missing_file_is_a_first_run_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let (settings, problem) = Settings::load(dir.path());
        assert_eq!(problem, Some(ConfigProblem::Absent));
        assert_eq!(settings.locale, "US");
    }

    #[test]
    fn the_shipped_template_parses_and_documents_the_defaults() {
        // The template is the first thing a person edits. If it does not parse,
        // every first run reports a broken config.
        let (settings, problem) = Settings::parse(Settings::template());
        assert_eq!(problem, None, "the template does not parse");
        assert_eq!(settings.weights.payout_for(Vendor::Bandcamp), 15);
    }
}
