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
