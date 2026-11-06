//! How Chrome starts: an ordinary Chrome with no automation markers, in the
//! user's time zone, languages and color scheme, with the live view's capture
//! extension loaded, and on Linux with the Chrome runtime and without its
//! sandbox.
//!
//! - Pages see no automation markers (`navigator.webdriver`, the headless
//!   token, hidden scrollbars), because the agent and the user test how real
//!   sites behave for real visitors.
//! - Linux runs without the sandbox: it needs unprivileged user namespaces,
//!   which Ubuntu 23.10 and later restrict, and Chrome refuses it as root. A
//!   compromised renderer then has the rights of the runner's user, which the
//!   agent already has through its shell. A Cloud runs inside gVisor besides.
//! - The browser's toolbar and address-bar WebUI are disabled: they are not
//!   agent pages and must not consume renderers.
//! - Time zone, languages and color scheme are the user's, taken from the
//!   starting invocation's command context, not the device's, which differs
//!   between devices. They apply to the whole browser as it starts, so every
//!   page, worker and request has them from its first byte, and a running
//!   browser keeps them.

use std::path::{Path, PathBuf};

use chromiumoxide::browser::BrowserConfigBuilder;

use demi_live_view_protocol::VIDEO_CODEC;
use demi_command_protocol::{ColorScheme, CommandLocale};

use crate::driver::operation::Result;

/// The capture extension's ID, fixed by the public key in its manifest so that
/// tab capture can allowlist it.
pub const CAPTURE_EXTENSION_ID: &str = "ekadkclcinpnbbdeloemlmaimcklplko";

/// Where the capture extension of the browser with `profile` lies.
pub fn capture_extension(profile: &Path) -> PathBuf {
    profile.join("demi-capture")
}

const CAPTURE_EXTENSION: &[(&str, &str)] = &[
    ("manifest.json", include_str!("capture/manifest.json")),
    ("background.js", include_str!("capture/background.js")),
    ("offscreen.html", include_str!("capture/offscreen.html")),
    ("offscreen.js", include_str!("capture/offscreen.js")),
];

/// An installed Chrome runtime, which Chrome starts with on Linux:
/// the directory of its libraries,
/// which holds the empty `gio/modules` too, and its fontconfig file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    pub libraries: PathBuf,
    pub fonts: PathBuf,
}

/// The variable Chrome does not inherit with the runtime: it names more of
/// the device's GIO modules, which `GIO_MODULE_DIR` does not replace.
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

    /// Chrome's environment with the runtime: its libraries before the
    /// device's, its fonts, and neither the device's crypto policy nor its GIO
    /// modules, which are built for the device's newer libraries.
    pub fn environment(&self) -> Vec<(String, String)> {
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

/// Chrome's switches beside the ones the driver owns (profile, debugging port,
/// extension, sandbox).
///
/// They are chromiumoxide's defaults without `enable-automation` and `lang`,
/// headless without hidden scrollbars, in the user's color scheme.
///
/// With the runtime, a desktop device's audio plugins would load beside its older
/// libraries, and the live view carries no sound: Chrome has no audio output.
fn switches(
    version: &str,
    color_scheme: ColorScheme,
    runtime: Option<&Runtime>,
) -> Vec<(String, Option<String>)> {
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
    switches.push(("blink-settings".into(), Some(blink_settings(color_scheme))));
    switches.push((
        "allowlisted-extension-id".into(),
        Some(CAPTURE_EXTENSION_ID.into()),
    ));
    if runtime.is_some() {
        switches.push(("disable-audio-output".into(), None));
    }
    switches
}

/// Blink's settings for every page and worker, which Chrome takes from its last
/// `blink-settings` switch only.
///
/// - The preferred color scheme is the user's, whatever the device's appearance.
///   `prefers-color-scheme` follows it, and Chrome paints an empty page in it,
///   `#121212` in the dark one (measured with the pinned Chrome on a dark Mac
///   told to be light, and the other way round).
/// - A headless Linux browser also reports no hover and a coarse pointer, and
///   pages would take their touch styles.
fn blink_settings(color_scheme: ColorScheme) -> String {
    // Blink's `PreferredColorScheme`: 0 is dark, 1 light.
    let scheme = match color_scheme {
        ColorScheme::Dark => 0,
        ColorScheme::Light => 1,
    };
    let mut settings = format!("preferredColorScheme={scheme}");
    if cfg!(target_os = "linux") {
        settings.push_str(
            ",primaryPointerType=4,availablePointerTypes=4,primaryHoverType=2,availableHoverTypes=2",
        );
    }
    settings
}

/// The Chrome major version pages see, from a release version such as
/// `153.0.8010.36`.
fn major(version: &str) -> &str {
    version.split('.').next().unwrap_or(version)
}

/// The standard user agent of Chrome on the device's platform, without the
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

/// Configures Chrome's switches, the user's locale and color scheme, and the
/// capture extension on a profile about to be launched.
///
/// The extension dials `capture` and encodes with the live protocol's codec.
///
/// With `runtime`, which a Linux device starts Chrome with, Chrome finds the
/// runtime through its own environment and runs without its sandbox. The
/// sandbox needs user namespaces, which Ubuntu restricts, and Chrome refuses it
/// as root.
pub async fn configure(
    builder: BrowserConfigBuilder,
    profile: &Path,
    version: &str,
    locale: &CommandLocale,
    color_scheme: ColorScheme,
    runtime: Option<&Runtime>,
    capture: &str,
) -> Result<BrowserConfigBuilder> {
    // A conversation's profile keeps the extension of an earlier start, which
    // may be another release's.
    let extension = capture_extension(profile);
    match tokio::fs::remove_dir_all(&extension).await {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
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
    // A conversation's profile keeps the preferences the sites and Chrome set
    // before, and only this one changes.
    let preferences = profile.join("Default");
    tokio::fs::create_dir_all(&preferences).await?;
    let file = preferences.join("Preferences");
    let mut kept = match tokio::fs::read(&file).await {
        Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({})),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(error) => return Err(error.into()),
    };
    if !kept["intl"].is_object() {
        kept["intl"] = serde_json::json!({});
    }
    kept["intl"]["accept_languages"] = serde_json::Value::String(locale.languages.join(","));
    // Replaced whole, so a start that ends halfway leaves the file as it was.
    let staged = preferences.join("Preferences.demi");
    tokio::fs::write(
        &staged,
        serde_json::to_vec(&kept).map_err(|error| {
            crate::driver::operation::BrowserError::Configuration(error.to_string())
        })?,
    )
    .await?;
    tokio::fs::rename(&staged, &file).await?;
    let mut builder = builder
        .disable_default_args()
        .with_head()
        .extension(extension.to_string_lossy().into_owned())
        .envs(environment(locale, runtime));
    if runtime.is_some() {
        builder = builder.no_sandbox();
    }
    for (name, value) in switches(version, color_scheme, runtime) {
        builder = match value {
            Some(value) => builder.arg((name.as_str(), value.as_str())),
            None => builder.arg(name),
        };
    }
    Ok(builder)
}

/// How the Chrome of a drawing starts (`crate::tabs::drawing`): with the same
/// switches, in the light scheme, which a drawn document's own theme
/// overrides, and with neither the capture extension nor profile preferences,
/// since it draws one document and ends.
pub fn drawing(
    builder: BrowserConfigBuilder,
    version: &str,
    runtime: Option<&Runtime>,
) -> BrowserConfigBuilder {
    let mut builder = builder
        .disable_default_args()
        .with_head()
        .envs(runtime.map(Runtime::environment).unwrap_or_default());
    if runtime.is_some() {
        builder = builder.no_sandbox();
    }
    for (name, value) in switches(version, ColorScheme::Light, runtime) {
        if name == "allowlisted-extension-id" {
            continue;
        }
        builder = match value {
            Some(value) => builder.arg((name.as_str(), value.as_str())),
            None => builder.arg(name),
        };
    }
    builder
}

/// Chrome's environment: the time zone every renderer uses, on Linux the
/// locale that sets a page's default formats, and the runtime's variables.
/// They are Chrome's alone: set for the runner, the runtime's libraries
/// would load into the device's own programs.
fn environment(locale: &CommandLocale, runtime: Option<&Runtime>) -> Vec<(String, String)> {
    let mut environment = vec![("TZ".to_owned(), locale.time_zone.clone())];
    if let Some(runtime) = runtime {
        environment.extend(runtime.environment());
    }
    if cfg!(target_os = "linux")
        && let Some(language) = locale.languages.first()
    {
        let system_locale = format!("{}.UTF-8", language.replace('-', "_"));
        // An inherited LC_ALL or LANGUAGE overrides LANG on paired Linux devices.
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
    locale: Option<&CommandLocale>,
    runtime: Option<&Runtime>,
) -> &'a mut tokio::process::Command {
    if cfg!(target_os = "macos")
        && let Some(language) = locale.and_then(|locale| locale.languages.first())
    {
        command.args(["-AppleLanguages".to_owned(), format!("({language})")]);
    }
    if runtime.is_some() {
        command.env_remove(GIO_EXTRA_MODULES);
    }
    command
}
