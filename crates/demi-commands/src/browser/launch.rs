//! How Chrome starts (`browser.md` § Native driver): an ordinary Chrome with no
//! automation markers, in the user's time zone and languages, with the live
//! view's capture extension loaded.

use std::path::Path;

use chromiumoxide::browser::BrowserConfigBuilder;
use demi_command_service::protocol::CommandLocale;

use super::Result;

/// The capture extension's ID, fixed by the public key in its manifest so that
/// tab capture can allowlist it (`browser-live-view.md` § Capture).
pub(super) const CAPTURE_EXTENSION_ID: &str = "ekadkclcinpnbbdeloemlmaimcklplko";

const CAPTURE_EXTENSION: &[(&str, &str)] = &[
    ("manifest.json", include_str!("capture/manifest.json")),
    ("background.js", include_str!("capture/background.js")),
    ("offscreen.html", include_str!("capture/offscreen.html")),
    ("offscreen.js", include_str!("capture/offscreen.js")),
];

/// The height the browser's own chrome adds to a window, so that a page never
/// sees an outer size smaller than its inner size.
pub(super) const WINDOW_CHROME_HEIGHT: u32 = 87;

/// Chrome's switches beside the ones the driver owns (profile, debugging
/// port, extension): chromiumoxide's defaults without `enable-automation` and
/// `lang`, headless without hidden scrollbars.
fn switches(version: &str) -> Vec<(String, Option<String>)> {
    let mut switches: Vec<(String, Option<String>)> = [
        "disable-background-networking",
        "disable-background-timer-throttling",
        "disable-backgrounding-occluded-windows",
        "disable-breakpad",
        "disable-client-side-phishing-detection",
        "disable-component-extensions-with-background-pages",
        "disable-default-apps",
        "disable-dev-shm-usage",
        "disable-hang-monitor",
        "disable-ipc-flooding-protection",
        "disable-popup-blocking",
        "disable-prompt-on-repost",
        "disable-renderer-backgrounding",
        "disable-sync",
        "metrics-recording-only",
        "no-first-run",
        "use-mock-keychain",
        "no-startup-window",
        "headless",
        "mute-audio",
    ]
    .into_iter()
    .map(|name| (name.to_owned(), None))
    .collect();
    for (name, value) in [
        ("enable-features", "NetworkService,NetworkServiceInProcess"),
        // Headless automation has no browser toolbar or omnibox. Avoid their
        // WebUI renderers and preload work, including Chrome's overhead trial.
        (
            "disable-features",
            "TranslateUI,InitialWebUI,WebUIToolbarProcessOverheadExperiment,PreloadTopChromeWebUI,WebUIOmniboxPopup,WebUIOmniboxAimPopup",
        ),
        ("force-color-profile", "srgb"),
        ("password-store", "basic"),
        ("enable-blink-features", "IdleDetection"),
        // `navigator.webdriver` stays false without `enable-automation` only
        // when this Blink feature is off too.
        ("disable-blink-features", "AutomationControlled"),
    ] {
        switches.push((name.to_owned(), Some(value.to_owned())));
    }
    switches.push(("user-agent".into(), Some(desktop_user_agent(version))));
    // A headless Linux browser reports no hover and a coarse pointer; pages
    // would take their touch styles.
    if cfg!(target_os = "linux") {
        switches.push((
            "blink-settings".into(),
            Some(
                "primaryPointerType=4,availablePointerTypes=4,primaryHoverType=2,availableHoverTypes=2"
                    .into(),
            ),
        ));
    }
    switches.push((
        "allowlisted-extension-id".into(),
        Some(CAPTURE_EXTENSION_ID.into()),
    ));
    switches
}

/// The Chrome major version pages see, from a release version such as
/// `153.0.8010.36`.
fn major(version: &str) -> &str {
    version.split('.').next().unwrap_or(version)
}

/// The standard user agent of Chrome on the Host's platform, without the
/// headless token; Chrome reports only the major version and a fixed platform.
fn desktop_user_agent(version: &str) -> String {
    let platform = if cfg!(target_os = "macos") {
        "Macintosh; Intel Mac OS X 10_15_7"
    } else if cfg!(windows) {
        "Windows NT 10.0; Win64; x64"
    } else {
        "X11; Linux x86_64"
    };
    format!(
        "Mozilla/5.0 ({platform}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{}.0.0.0 Safari/537.36",
        major(version)
    )
}

/// Configures Chrome's switches, the user's locale and the capture extension
/// on a profile about to be launched; the extension dials `capture`.
pub(super) async fn configure(
    builder: BrowserConfigBuilder,
    profile: &Path,
    version: &str,
    locale: &CommandLocale,
    capture: &str,
) -> Result<BrowserConfigBuilder> {
    let extension = profile.join("demi-capture");
    tokio::fs::create_dir(&extension).await?;
    for (name, contents) in CAPTURE_EXTENSION {
        tokio::fs::write(extension.join(name), contents).await?;
    }
    tokio::fs::write(
        extension.join("config.js"),
        format!(
            "export const socket = {};\n",
            serde_json::Value::String(capture.to_owned())
        ),
    )
    .await?;
    // The profile's language preference sets `navigator.languages` and every
    // request's `Accept-Language` from the first byte, in popups and workers too.
    let preferences = profile.join("Default");
    tokio::fs::create_dir(&preferences).await?;
    tokio::fs::write(
        preferences.join("Preferences"),
        serde_json::to_vec(&serde_json::json!({
            "intl": { "accept_languages": locale.languages.join(",") }
        }))
        .map_err(|error| super::BrowserError::Configuration(error.to_string()))?,
    )
    .await?;
    let mut builder = builder
        .disable_default_args()
        .with_head()
        .extension(extension.to_string_lossy().into_owned())
        .envs(environment(locale));
    for (name, value) in switches(version) {
        builder = match value {
            Some(value) => builder.arg((name.as_str(), value.as_str())),
            None => builder.arg(name),
        };
    }
    Ok(builder)
}

/// Chrome's environment: the time zone every renderer uses, and on Linux the
/// locale that sets a page's default formats.
fn environment(locale: &CommandLocale) -> Vec<(String, String)> {
    let mut environment = vec![("TZ".to_owned(), locale.time_zone.clone())];
    if cfg!(target_os = "linux")
        && let Some(language) = locale.languages.first()
    {
        environment.push((
            "LANG".to_owned(),
            format!("{}.UTF-8", language.replace('-', "_")),
        ));
    }
    environment
}

/// Arguments Chrome takes in the platform's own form: on macOS the first
/// language as the application language, which sets a page's default formats.
pub(super) fn platform_arguments(locale: &CommandLocale) -> Vec<String> {
    match locale.languages.first() {
        Some(language) if cfg!(target_os = "macos") => {
            vec!["-AppleLanguages".to_owned(), format!("({language})")]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use sha2::{Digest, Sha256};

    use super::*;

    /// Chrome derives an unpacked extension's ID from its manifest key: the
    /// first 16 bytes of the key's SHA-256, each hex digit as `a` to `p`.
    #[test]
    fn the_capture_extension_id_is_its_manifest_key() {
        let manifest: serde_json::Value = serde_json::from_str(CAPTURE_EXTENSION[0].1).unwrap();
        let key = base64::engine::general_purpose::STANDARD
            .decode(manifest["key"].as_str().unwrap())
            .unwrap();
        let id: String = Sha256::digest(key)[..16]
            .iter()
            .flat_map(|byte| [byte >> 4, byte & 0xf])
            .map(|nibble| char::from(b'a' + nibble))
            .collect();
        assert_eq!(id, CAPTURE_EXTENSION_ID);
    }

    #[test]
    fn the_user_agent_carries_the_major_version_and_no_headless_token() {
        let desktop = desktop_user_agent("153.0.8010.36");
        assert!(desktop.contains("Chrome/153.0.0.0"), "{desktop}");
        assert!(!desktop.contains("Headless"));
    }
}
