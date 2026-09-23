//! Where a request of an API-key entry goes (`providers.md` § Endpoints).

use reqwest::Url;

/// The URL of the request at `path`, such as `/responses`, under an entry's
/// API `base`: the base with `path` appended, unless the base already ends
/// with it. A trailing slash of the base is ignored.
pub fn endpoint_url(base: &Url, path: &str) -> Url {
    let mut url = base.clone();
    let trimmed = url.path().trim_end_matches('/').to_owned();
    if trimmed.ends_with(path) {
        url.set_path(&trimmed);
    } else {
        url.set_path(&format!("{trimmed}{path}"));
    }
    url
}
