#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;

use demi_command_package_browser_chrome::driver::{
    installation::Installation, launch::Runtime, numbers::TabNumbers,
};
use demi_command_package_browser_chrome::tabs::environment::{LaunchOptions, with_browser};
use demi_command_sdk::testing::counting_numbers;
use tokio_util::sync::CancellationToken;

/// What Chrome's process was started with: its arguments and its
/// environment, as a launcher in its place recorded them.
struct Started {
    arguments: Vec<String>,
    environment: Vec<(String, String)>,
}

impl Started {
    fn variable(&self, name: &str) -> Option<&str> {
        self.environment
            .iter()
            .find(|(variable, _)| variable == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Starts a launcher in Chrome's place with `runtime`, which records what
/// it was started with and exits, so the launch fails.
async fn start(runtime: Option<Runtime>) -> Started {
    let directory = tempfile::tempdir().unwrap();
    let launcher = directory.path().join("chrome");
    std::fs::write(
        &launcher,
        "#!/bin/sh\n{ printf '%s\\n' \"$@\"; echo --; env; } > \"$0.record\"\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
    let locale = demi_command_protocol::CommandLocale {
        time_zone: "UTC".into(),
        languages: vec!["en-US".into()],
    };
    let installation = Installation {
        executable: launcher.clone(),
        runtime,
    };
    let launched = with_browser(
        LaunchOptions::pinned(installation, locale, demi_command_protocol::ColorScheme::Light).unwrap(),
        TabNumbers::new(counting_numbers(), "conversation".into()),
        CancellationToken::new(),
        |_| async { Ok(()) },
    )
    .await;
    assert!(launched.is_err(), "the launcher is no Chrome");
    let record = std::fs::read_to_string(directory.path().join("chrome.record")).unwrap();
    let (arguments, environment) = record.split_once("--\n").unwrap();
    Started {
        arguments: arguments.lines().map(str::to_owned).collect(),
        environment: environment
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
    }
}

/// On Linux Chrome starts with the Chrome runtime: its own environment
/// names the runtime's libraries and fonts, keeps out the Host's crypto
/// policy and GIO modules, and it runs without its sandbox and audio
/// output. Without the runtime, as on macOS and Windows, it has none of
/// that and keeps its sandbox. The runner's own environment never gets the
/// variables (`browser.md` § Browser distribution, § Native driver).
// Cost: about 0.1 s; a shell script stands in for Chrome.
#[tokio::test]
async fn chrome_alone_starts_with_the_runtime_and_without_its_sandbox() {
    let variables = [
        "LD_LIBRARY_PATH",
        "FONTCONFIG_FILE",
        "NSS_IGNORE_SYSTEM_POLICY",
        "GIO_MODULE_DIR",
    ];
    let flags = ["--no-sandbox", "--disable-audio-output"];
    let runtime = Runtime::unpacked(std::path::Path::new("/opt/runtime"));
    let started = start(Some(runtime)).await;
    assert_eq!(started.variable("LD_LIBRARY_PATH"), Some("/opt/runtime/lib"));
    assert_eq!(
        started.variable("FONTCONFIG_FILE"),
        Some("/opt/runtime/fontconfig/fonts.conf")
    );
    assert_eq!(started.variable("NSS_IGNORE_SYSTEM_POLICY"), Some("1"));
    assert_eq!(
        started.variable("GIO_MODULE_DIR"),
        Some("/opt/runtime/lib/gio/modules")
    );
    for flag in flags {
        assert!(started.arguments.iter().any(|argument| argument == flag), "{flag}");
    }
    for variable in &variables[1..] {
        assert_eq!(std::env::var_os(variable), None, "{variable}");
    }

    let started = start(None).await;
    // Cargo sets the library path of a Linux test process, which Chrome
    // inherits as any child does.
    let inherited = std::env::var("LD_LIBRARY_PATH").ok();
    assert_eq!(started.variable("LD_LIBRARY_PATH"), inherited.as_deref());
    for variable in &variables[1..] {
        assert_eq!(started.variable(variable), None, "{variable}");
    }
    for flag in flags {
        assert!(!started.arguments.iter().any(|argument| argument == flag), "{flag}");
    }
}
