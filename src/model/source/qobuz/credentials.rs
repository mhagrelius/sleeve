//! Getting an `app_id` out of the Qobuz web player.
//!
//! Qobuz publishes no API keys. Its web player carries one in its JavaScript
//! bundle, and every third-party client works the same way: load the player,
//! find the bundle, read the id out of it.
//!
//! Scope matters here. The catalogue endpoints this application uses — search,
//! and album lookup — need only the `app_id`. The `app_secret` and the signed
//! request that goes with it are for `track/getFileUrl`, which produces a
//! streaming URL for the audio itself; Sleeve never calls it and never wants to.
//! So the credential handling here stops at the id, which keeps the footprint to
//! "read a catalogue the way the website reads it".
//!
//! The id rotates when Qobuz redeploys, so it is cached with a long TTL and
//! refreshed when the API answers 401. That refresh is a separate operation with
//! its own failure mode, never part of a lookup.

use super::super::{Outcome, Reason, Request, SourceId};

/// The player page, which references the bundle that holds the id.
pub fn player_page() -> Request {
    Request::get(SourceId::QobuzStore, "https://play.qobuz.com/login").header("Accept", "text/html")
}

/// One of the player's JavaScript bundles.
pub fn bundle(path: &str) -> Request {
    let url = if path.starts_with("http") {
        path.to_string()
    } else {
        format!("https://play.qobuz.com{path}")
    };
    Request::get(SourceId::QobuzStore, url).header("Accept", "application/javascript")
}

/// Find the bundle URL in the player page.
pub fn parse_bundle_path(body: &[u8]) -> Outcome<String> {
    let text = String::from_utf8_lossy(body);
    // `<script src="/resources/N.N.N-bNNNNN/bundle.js">`
    let Some(start) = text.find("/resources/") else {
        return Outcome::Stale(Reason::Malformed(
            "no bundle reference in the Qobuz player page".into(),
        ));
    };
    let rest = &text[start..];
    let Some(end) = rest.find("bundle.js") else {
        return Outcome::Stale(Reason::Malformed("no bundle.js in the player page".into()));
    };
    Outcome::Found(rest[..end + "bundle.js".len()].to_string())
}

/// Read the `app_id` out of a bundle.
///
/// The bundle is minified, so this looks for the key rather than for any
/// surrounding structure — a layout-sensitive parse would break on every
/// redeploy, and the key itself has been stable for years.
pub fn parse_app_id(body: &[u8]) -> Outcome<String> {
    let text = String::from_utf8_lossy(body);
    // The live bundle spells it `production:{api:{appId:"798273057"`. The
    // snake-case spellings are what every third-party client documents, and both
    // appear in the wild depending on which bundle you land on, so all of them
    // are tried rather than whichever one was true the day this was written.
    for needle in [
        "appId:\"",
        "\"appId\":\"",
        "app_id:\"",
        "\"app_id\":\"",
        "app_id=\"",
    ] {
        if let Some(start) = text.find(needle) {
            let rest = &text[start + needle.len()..];
            if let Some(end) = rest.find('"') {
                let id = &rest[..end];
                // The id is numeric. Anything else means the needle matched
                // something that merely looks like it.
                if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
                    return Outcome::Found(id.to_string());
                }
            }
        }
    }
    Outcome::Stale(Reason::Malformed(
        "no app_id in the Qobuz player bundle".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_path_is_found_in_the_player_page() {
        let page = br#"<html><head>
            <script src="/resources/7.1.3-b011/bundle.js"></script>
            </head></html>"#;
        assert_eq!(
            parse_bundle_path(page),
            Outcome::Found("/resources/7.1.3-b011/bundle.js".into())
        );
    }

    #[test]
    fn an_app_id_is_read_out_of_a_minified_bundle_in_any_of_its_spellings() {
        for body in [
            br#"...,production:{api:{appId:"x",app_id:"798273057"}},..."#.as_slice(),
            br#"{"app_id":"798273057","other":1}"#.as_slice(),
            br#"var c=app_id="798273057";"#.as_slice(),
        ] {
            assert_eq!(parse_app_id(body), Outcome::Found("798273057".into()));
        }
    }

    #[test]
    fn a_non_numeric_match_is_rejected_rather_than_used() {
        // Matching the wrong thing would send every catalogue request with a
        // junk id and produce a wall of 401s that looks like an outage.
        assert!(matches!(
            parse_app_id(br#"app_id:"undefined""#),
            Outcome::Stale(_)
        ));
    }

    #[test]
    fn a_redesigned_player_is_stale_rather_than_silently_empty() {
        assert!(matches!(
            parse_bundle_path(b"<html></html>"),
            Outcome::Stale(_)
        ));
        assert!(matches!(parse_app_id(b"nothing here"), Outcome::Stale(_)));
    }

    #[test]
    fn a_bundle_path_becomes_an_absolute_url() {
        assert_eq!(
            bundle("/resources/7.1.3-b011/bundle.js").url,
            "https://play.qobuz.com/resources/7.1.3-b011/bundle.js"
        );
        // An already-absolute one is left alone.
        assert_eq!(
            bundle("https://elsewhere/b.js").url,
            "https://elsewhere/b.js"
        );
    }
}
