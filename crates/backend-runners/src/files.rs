//! What the product reads from a Host's files for the web app: directory
//! listings and file text (`web-api.md` § Device files and remote
//! references, § File text and working tree changes). Text is what edit
//! tracking and line counts read as text, UTF-8 without a NUL byte, up to
//! the size an edit snapshot keeps, so a file the runner counted lines for
//! is one the web app shows.

use bytes::{Bytes, BytesMut};
use demi_command_protocol::EDIT_FILE_BYTES;
use demi_command_protocol::is_text;
use demi_host_interface::{ByteRange, ByteStream, FileKind, HostError, HostFs};
use futures_util::StreamExt as _;
use demi_web_api_protocol::files::DirectoryEntry;

/// Why a file is not shown as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TextRefusal {
    #[error("The file is too large to show")]
    TooLarge,
    #[error("The file is not UTF-8 text")]
    NotText,
}

/// Why a file's text could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    #[error(transparent)]
    Host(#[from] HostError),
    #[error(transparent)]
    Refused(#[from] TextRefusal),
}

/// `bytes` as text, under the limits above.
pub fn text_of(bytes: Bytes) -> Result<String, TextRefusal> {
    if bytes.len() > EDIT_FILE_BYTES {
        return Err(TextRefusal::TooLarge);
    }
    if !is_text(&bytes) {
        return Err(TextRefusal::NotText);
    }
    // `is_text` checked the encoding.
    Ok(String::from_utf8(bytes.to_vec()).expect("text is UTF-8"))
}

/// The range a text read asks for: one byte more than the limit, so a file
/// whose bytes go past it is refused once that byte arrives.
pub const TEXT_RANGE: ByteRange = ByteRange {
    offset: 0,
    length: Some(EDIT_FILE_BYTES as u64 + 1),
};

/// One file of a Host as text, with one request (`runner.md` § Host
/// operations), read as [`TEXT_RANGE`] says.
pub async fn read_text_file(fs: &dyn HostFs, path: &str) -> Result<String, TextError> {
    text_of_stream(fs.read_stream(path, TEXT_RANGE).await?).await
}

/// The bytes of a [`TEXT_RANGE`] read as text, under the limits above.
pub async fn text_of_stream(mut stream: ByteStream) -> Result<String, TextError> {
    let mut bytes = BytesMut::new();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk?);
        if bytes.len() > EDIT_FILE_BYTES {
            return Err(TextRefusal::TooLarge.into());
        }
    }
    Ok(text_of(bytes.freeze())?)
}

/// A directory's entries with their metadata, which the listing carries,
/// so the whole answer is one request to the Host (`runner.md` § Host
/// operations).
pub async fn browse_directory(
    fs: &dyn HostFs,
    path: &str,
) -> Result<Vec<DirectoryEntry>, HostError> {
    let entries = fs.read_dir(path).await?;
    Ok(entries
        .into_iter()
        .map(|entry| DirectoryEntry {
            name: entry.name,
            is_directory: entry.kind == FileKind::Directory,
            is_symbolic_link: entry.kind == FileKind::Symlink,
            size: entry.size,
            modified_at: entry.modified,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use demi_backend_remote_host::testing::{FixtureOptions, RunnerFixture, answered_requests};
    use tokio::sync::mpsc;

    use super::*;

    /// The modification time `path` itself has, in milliseconds.
    fn modified(path: &std::path::Path) -> i64 {
        let time = std::fs::symlink_metadata(path).unwrap().modified().unwrap();
        let milliseconds = time.duration_since(UNIX_EPOCH).unwrap().as_millis();
        i64::try_from(milliseconds).unwrap()
    }

    // About a tenth of a second: a real runner process lists the directory.
    #[tokio::test(flavor = "local")]
    async fn a_listing_is_one_request_to_the_host_and_carries_each_entrys_own_metadata() {
        let (tap, mut replies) = mpsc::channel(1 << 10);
        let fixture = RunnerFixture::start(FixtureOptions {
            tap: Some(tap),
            ..FixtureOptions::default()
        })
        .await;
        let directory = fixture.home_dir().join("listed");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("notes.txt"), "four").unwrap();
        let written = UNIX_EPOCH + Duration::from_millis(1_600_000_000_123);
        std::fs::File::options()
            .write(true)
            .open(directory.join("notes.txt"))
            .unwrap()
            .set_modified(written)
            .unwrap();
        std::fs::create_dir(directory.join("photos")).unwrap();
        // A link's own size is its target's name, nine bytes, not the four
        // the target holds.
        std::os::unix::fs::symlink("notes.txt", directory.join("link")).unwrap();
        answered_requests(&mut replies);

        let host = fixture.host();
        let path = format!("{}/listed", fixture.home());
        let entries = browse_directory(&host, &path).await.unwrap();

        assert_eq!(
            answered_requests(&mut replies),
            1,
            "one request lists the directory"
        );
        let mut listed: Vec<_> = entries
            .iter()
            .map(|entry| {
                (
                    entry.name.as_str(),
                    entry.is_directory,
                    entry.is_symbolic_link,
                    entry.size,
                    entry.modified_at.as_millisecond(),
                )
            })
            .collect();
        listed.sort_unstable();
        let photos = directory.join("photos");
        assert_eq!(
            listed,
            [
                ("link", false, true, 9, modified(&directory.join("link"))),
                ("notes.txt", false, false, 4, 1_600_000_000_123),
                (
                    "photos",
                    true,
                    false,
                    std::fs::symlink_metadata(&photos).unwrap().len(),
                    modified(&photos),
                ),
            ]
        );
        fixture.stop().await;
    }

    // About a tenth of a second: a real runner process reads the files.
    #[tokio::test(flavor = "local")]
    async fn a_files_text_is_one_request_and_a_file_past_the_limit_is_refused_by_its_bytes() {
        let (tap, mut replies) = mpsc::channel(1 << 10);
        let fixture = RunnerFixture::start(FixtureOptions {
            tap: Some(tap),
            ..FixtureOptions::default()
        })
        .await;
        std::fs::write(fixture.home_dir().join("notes.txt"), "1\n2\n").unwrap();
        std::fs::write(
            fixture.home_dir().join("large.txt"),
            vec![b'a'; EDIT_FILE_BYTES + 1],
        )
        .unwrap();
        answered_requests(&mut replies);
        let host = fixture.host();

        let notes = read_text_file(&host, &format!("{}/notes.txt", fixture.home())).await;
        assert_eq!(notes, Ok("1\n2\n".to_owned()));
        assert_eq!(answered_requests(&mut replies), 1, "one request reads the text");
        let large = read_text_file(&host, &format!("{}/large.txt", fixture.home())).await;
        assert_eq!(large, Err(TextError::Refused(TextRefusal::TooLarge)));
        assert_eq!(answered_requests(&mut replies), 1, "one request refuses it");
        fixture.stop().await;
    }

    #[test]
    fn text_is_utf8_without_a_nul_byte_up_to_the_snapshot_limit() {
        assert_eq!(
            text_of(Bytes::from_static(b"1\n2\n")),
            Ok("1\n2\n".to_owned())
        );
        assert_eq!(
            text_of(Bytes::from_static(b"\x00\xff\x01")),
            Err(TextRefusal::NotText)
        );
        assert_eq!(
            text_of(Bytes::from_static(b"nul \x00 inside")),
            Err(TextRefusal::NotText)
        );
        assert_eq!(
            text_of(Bytes::from(vec![b'a'; EDIT_FILE_BYTES + 1])),
            Err(TextRefusal::TooLarge)
        );
    }
}
