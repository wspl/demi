//! The browser and system a request's User-Agent names, which name a
//! signed-in browser and a passkey no password manager names.

/// The browser and system a User-Agent names, such as `Safari on iPhone`.
///
/// - The result names their families, never their versions, in the order that
///   tells them apart: Edge's User-Agent names Chrome and Safari too, and
///   Chrome's names Safari.
/// - The result names an iPhone and an iPad by the device, as the product names
///   them.
/// - The result is one family alone when the User-Agent names only one, and
///   none when it names neither. The caller names it then.
pub fn browser_and_system(user_agent: &str) -> Option<String> {
    const BROWSERS: [(&[&str], &str); 6] = [
        (&["Edg/", "EdgA/", "EdgiOS/"], "Edge"),
        (&["OPR/"], "Opera"),
        (&["SamsungBrowser/"], "Samsung Internet"),
        (&["Firefox/", "FxiOS/"], "Firefox"),
        (&["Chrome/", "CriOS/"], "Chrome"),
        (&["Safari/"], "Safari"),
    ];
    const SYSTEMS: [(&[&str], &str); 7] = [
        (&["iPhone"], "iPhone"),
        (&["iPad"], "iPad"),
        (&["Android"], "Android"),
        (&["CrOS"], "ChromeOS"),
        (&["Windows"], "Windows"),
        (&["Macintosh"], "macOS"),
        (&["Linux"], "Linux"),
    ];
    let find = |families: &[(&[&str], &'static str)]| {
        families
            .iter()
            .find(|(marks, _)| marks.iter().any(|mark| user_agent.contains(mark)))
            .map(|(_, name)| *name)
    };
    match (find(&BROWSERS), find(&SYSTEMS)) {
        (Some(browser), Some(system)) => Some(format!("{browser} on {system}")),
        (Some(name), None) | (None, Some(name)) => Some(name.to_owned()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_agent_names_its_browser_and_system() {
        for (user_agent, name) in [
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15",
                Some("Safari on macOS"),
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
                Some("Chrome on Windows"),
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0",
                Some("Edge on Windows"),
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1",
                Some("Safari on iPhone"),
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/141.0 Mobile/15E148 Safari/604.1",
                Some("Chrome on iPhone"),
            ),
            (
                "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0",
                Some("Firefox on Linux"),
            ),
            (
                "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36",
                Some("Chrome on Android"),
            ),
            ("curl/8.7.1", None),
        ] {
            assert_eq!(browser_and_system(user_agent).as_deref(), name, "{user_agent}");
        }
    }
}
