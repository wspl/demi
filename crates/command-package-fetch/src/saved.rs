//! Where the package keeps what it fetched, under the Host's temporary
//! directory's `demi-fetch/`: the pages read, and each file that is no text
//! in a directory named after its URL, `demi-fetch/<hash>/<name>`, so two
//! URLs with the same file name never overwrite each other (`web.md`
//! § demi web fetch).

use std::path::{Path, PathBuf};

use demi_command_protocol::canonical_digest;
use percent_encoding::percent_decode_str;
use reqwest::Url;

/// How many hexadecimal digits of its URL's digest a file's directory
/// carries.
const DIRECTORY_DIGITS: usize = 12;

/// The name of a file whose URL names none.
const UNNAMED: &str = "download";

/// The package's directory.
pub fn directory() -> PathBuf {
    std::env::temp_dir().join("demi-fetch")
}

/// The SHA-256 of `url`, which names what is kept of it.
pub fn digest(url: &str) -> String {
    canonical_digest(&url).expect("a string serializes")
}

/// Saves `bytes`, fetched from `url` whose last redirect reached `final_url`,
/// with the media type `media_type`, and returns its path.
pub async fn save(
    url: &str,
    final_url: &Url,
    media_type: Option<&str>,
    bytes: &[u8],
) -> std::io::Result<PathBuf> {
    let directory = directory().join(&digest(url)[..DIRECTORY_DIGITS]);
    tokio::fs::create_dir_all(&directory).await?;
    let path = directory.join(name(final_url, media_type));
    let partial = directory.join(".partial");
    tokio::fs::write(&partial, bytes).await?;
    tokio::fs::rename(&partial, &path).await?;
    Ok(path)
}

/// The file's name: the last segment of the URL's path, decoded, with the
/// extension of its media type when it has none.
fn name(url: &Url, media_type: Option<&str>) -> String {
    let segment = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .map(|segment| percent_decode_str(segment).decode_utf8_lossy().into_owned())
        .unwrap_or_default();
    // Separators and control characters would reach outside the directory
    // or break a shell line.
    let name: String = segment
        .chars()
        .map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c })
        .collect();
    let name = match name.trim_start_matches('.') {
        "" => UNNAMED.to_owned(),
        name => name.to_owned(),
    };
    if Path::new(&name).extension().is_some() {
        return name;
    }
    match media_type
        .and_then(mime_guess::get_mime_extensions_str)
        .and_then(|extensions| extensions.first())
    {
        Some(extension) => format!("{name}.{extension}"),
        None => name,
    }
}
