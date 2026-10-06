//! How Chrome starts (`browser.md` § Native driver): an ordinary Chrome with no
//! automation markers, in the user's time zone and languages, with the live
//! view's capture extension loaded, and on Linux with the Chrome runtime and
//! without its sandbox.

use std::path::{Path, PathBuf};

use chromiumoxide::browser::BrowserConfigBuilder;

use demi_command_package_browser_protocol::live::VIDEO_CODEC;
use demi_command_package_browser_protocol::release::{
    RUNTIME_FONTS_ENTRY, RUNTIME_LIBRARIES_ENTRY,
};
use demi_command_protocol::CommandLocale;

use crate::driver::operation::Result;

/// The capture extension's ID, fixed by the public key in its manifest so that
/// tab capture can allowlist it (`live-view.md` § Capture).
pub const CAPTURE_EXTENSION_ID: &str = "ekadkclcinpnbbdeloemlmaimcklplko";

const CAPTURE_EXTENSION: &[(&str, &str)] = &[
    ("manifest.json", include_str!("capture/manifest.json")),
    ("background.js", include_str!("capture/background.js")),
    ("offscreen.html", include_str!("capture/offscreen.html")),
    ("offscreen.js", include_str!("capture/offscreen.js")),
];

/// The height the browser's own chrome adds to a window, so that a page never
/// sees an outer size smaller than its inner size.
pub const WINDOW_CHROME_HEIGHT: u32 = 87;

/// An installed Chrome runtime, which Chrome starts with on Linux
/// (`browser.md` § Browser distribution): the directory of its libraries,
/// which holds the empty `gio/modules` too, and its fontconfig file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    pub libraries: PathBuf,
    pub fonts: PathBuf,
}

/// The variable Chrome does not inherit with the runtime: it names more of
/// the Host's GIO modules, which `GIO_MODULE_DIR` does not replace.
const GIO_EXTRA_MODULES: &str = "GIO_EXTRA_MODULES";

impl Runtime {
    /// The runtime whose archives' entries are installed at `libraries`
    /// and `fonts`.
    pub fn installed(libraries: &Path, fonts: PathBuf) -> Self {
        Self {
            libraries: libraries
                .parent()
                .expect("the libraries' entry lies in their directory")
                .to_owned(),
            fonts,
        }
    }

    /// The runtime with both archives unpacked into `directory`, as the
    /// Chrome tests' `DEMI_TEST_CHROME_RUNTIME` names it.
    pub fn unpacked(directory: &Path) -> Self {
        Self::installed(
            &directory.join(RUNTIME_LIBRARIES_ENTRY),
            directory.join(RUNTIME_FONTS_ENTRY),
        )
    }

    /// Chrome's environment with the runtime: its libraries before the
    /// Host's, its fonts, and neither the Host's crypto policy nor its GIO
    /// modules, which are built for the Host's newer libraries.
    fn environment(&self) -> Vec<(String, String)> {
        let path = |path: &Path| path.to_string_lossy().into_owned();
        vec![
            ("LD_LIBRARY_PATH".to_owned(), path(&self.libraries)),
            ("FONTCONFIG_FILE".to_owned(), path(&self.fonts)),
            ("NSS_IGNORE_SYSTEM_POLICY".to_owned(), "1".to_owned()),
            (
                "GIO_MODULE_DIR".to_owned(),
                path(&self.libraries.join("gio").join("modules")),
            ),
        ]
    }
}

/// Chrome's switches beside the ones the driver owns (profile, debugging
/// port, extension, sandbox): chromiumoxide's defaults without
/// `enable-automation` and `lang`, headless without hidden scrollbars. With
/// the runtime, a desktop Host's audio plugins would load beside its older
/// libraries, and the live view carries no sound: Chrome has no audio
/// output.
fn switches(version: &str, runtime: Option<&Runtime>) -> Vec<(String, Option<String>)> {
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
        // Reload reclassifies the capture extension as unpacked. Allow it in
        // this owned process without relying on a protected developer-mode
        // preference that Chrome can reset on macOS and Windows.
        (
            "disable-features",
            "TranslateUI,InitialWebUI,WebUIToolbarProcessOverheadExperiment,PreloadTopChromeWebUI,WebUIOmniboxPopup,WebUIOmniboxAimPopup,ExtensionDisableUnsupportedDeveloper",
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
    if runtime.is_some() {
        switches.push(("disable-audio-output".into(), None));
    }
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
/// on a profile about to be launched; the extension dials `capture` and
/// encodes with the live protocol's codec. With `runtime`, which a Linux
/// Host starts Chrome with, Chrome finds the runtime through its own
/// environment and runs without its sandbox: the sandbox needs user
/// namespaces, which Ubuntu restricts, and Chrome refuses it as root
/// (`browser.md` § Native driver).
pub async fn configure(
    builder: BrowserConfigBuilder,
    profile: &Path,
    version: &str,
    locale: &CommandLocale,
    runtime: Option<&Runtime>,
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
            "export const socket = {};\nexport const codec = {};\n",
            serde_json::Value::String(capture.to_owned()),
            serde_json::Value::String(VIDEO_CODEC.to_owned()),
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
        .map_err(|error| {
            crate::driver::operation::BrowserError::Configuration(error.to_string())
        })?,
    )
    .await?;
    let mut builder = builder
        .disable_default_args()
        .with_head()
        .extension(extension.to_string_lossy().into_owned())
        .envs(environment(locale, runtime));
    if runtime.is_some() {
        builder = builder.no_sandbox();
    }
    for (name, value) in switches(version, runtime) {
        builder = match value {
            Some(value) => builder.arg((name.as_str(), value.as_str())),
            None => builder.arg(name),
        };
    }
    Ok(builder)
}

/// Chrome's environment: the time zone every renderer uses, on Linux the
/// locale that sets a page's default formats, and the runtime's variables.
/// They are Chrome's alone: set for the runner, the runtime's libraries
/// would load into the Host's own programs.
fn environment(locale: &CommandLocale, runtime: Option<&Runtime>) -> Vec<(String, String)> {
    let mut environment = vec![("TZ".to_owned(), locale.time_zone.clone())];
    if let Some(runtime) = runtime {
        environment.extend(runtime.environment());
    }
    if cfg!(target_os = "linux")
        && let Some(language) = locale.languages.first()
    {
        let system_locale = format!("{}.UTF-8", language.replace('-', "_"));
        // An inherited LC_ALL or LANGUAGE overrides LANG on paired Linux Hosts.
        environment.push(("LANG".to_owned(), system_locale.clone()));
        environment.push(("LC_ALL".to_owned(), system_locale));
        environment.push(("LANGUAGE".to_owned(), locale.languages.join(":")));
    }
    environment
}

/// Completes Chrome's command beyond what chromiumoxide's configuration
/// can say: the arguments Chrome takes in the platform's own form, on macOS
/// the first language as the application language, which sets a page's
/// default formats; and, with the runtime, the inherited variable Chrome
/// must not see, since the configuration only adds variables.
pub fn complete<'a>(
    command: &'a mut tokio::process::Command,
    locale: &CommandLocale,
    runtime: Option<&Runtime>,
) -> &'a mut tokio::process::Command {
    if cfg!(target_os = "macos")
        && let Some(language) = locale.languages.first()
    {
        command.args(["-AppleLanguages".to_owned(), format!("({language})")]);
    }
    if runtime.is_some() {
        command.env_remove(GIO_EXTRA_MODULES);
    }
    command
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
