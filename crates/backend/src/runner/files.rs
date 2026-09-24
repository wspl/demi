//! What the product reads from a Host's files for the browser: directory
//! listings and file text (`web-api.md` § Device files and remote
//! references, § File text and working tree changes). Text is what edit
//! tracking and line counts read as text, UTF-8 without a NUL byte, up to
//! the size an edit snapshot keeps, so a file the runner counted lines for
//! is one the browser shows.

use bytes::Bytes;
use demi_command_service::edits::is_text;
use demi_command_service::protocol::EDIT_FILE_BYTES;
use demi_shell::{FileKind, HostError, HostFs};
use demi_web_api::files::DirectoryEntry;

/// Why a file is not shown as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(not(test), expect(dead_code, reason = "the conversation file routes read text with it"))]
pub(crate) enum TextRefusal {
    #[error("The file is too large to show")]
    TooLarge,
    #[error("The file is not UTF-8 text")]
    NotText,
}

/// Why a file's text could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[expect(dead_code, reason = "the conversation file routes read text with it")]
pub(crate) enum TextError {
    #[error(transparent)]
    Host(#[from] HostError),
    #[error(transparent)]
    Refused(#[from] TextRefusal),
}

/// `bytes` as text, under the limits above.
#[cfg_attr(not(test), expect(dead_code, reason = "the conversation file routes read text with it"))]
pub(crate) fn text_of(bytes: Bytes) -> Result<String, TextRefusal> {
    if bytes.len() > EDIT_FILE_BYTES {
        return Err(TextRefusal::TooLarge);
    }
    if !is_text(&bytes) {
        return Err(TextRefusal::NotText);
    }
    // `is_text` checked the encoding.
    Ok(String::from_utf8(bytes.to_vec()).expect("text is UTF-8"))
}

/// One file of a Host as text; a file over the limit is refused before its
/// bytes are read.
#[expect(dead_code, reason = "the conversation file routes read text with it")]
pub(crate) async fn read_text_file(fs: &dyn HostFs, path: &str) -> Result<String, TextError> {
    let stat = fs.stat(path).await?;
    if stat.size > EDIT_FILE_BYTES as u64 {
        return Err(TextRefusal::TooLarge.into());
    }
    Ok(text_of(fs.read_file(path).await?)?)
}

/// A directory's entries with their metadata. Each entry's metadata is
/// awaited before the next is asked for, so one listing cannot flood the
/// runner's queue; an entry that disappears meanwhile is left out.
pub(crate) async fn browse_directory(fs: &dyn HostFs, path: &str) -> Result<Vec<DirectoryEntry>, HostError> {
    let names = fs.read_dir(path).await?;
    let directory = path.trim_end_matches('/');
    let mut entries = Vec::with_capacity(names.len());
    for entry in names {
        let stat = match fs.lstat(&format!("{directory}/{}", entry.name)).await {
            Ok(stat) => stat,
            Err(error) if error.code() == Some("ENOENT") => continue,
            Err(error) => return Err(error),
        };
        entries.push(DirectoryEntry {
            name: entry.name,
            is_directory: entry.kind == FileKind::Directory,
            is_symbolic_link: stat.kind == FileKind::Symlink,
            size: stat.size,
            modified_at: stat.modified,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_utf8_without_a_nul_byte_up_to_the_snapshot_limit() {
        assert_eq!(text_of(Bytes::from_static(b"1\n2\n")), Ok("1\n2\n".to_owned()));
        assert_eq!(text_of(Bytes::from_static(b"\x00\xff\x01")), Err(TextRefusal::NotText));
        assert_eq!(text_of(Bytes::from_static(b"nul \x00 inside")), Err(TextRefusal::NotText));
        assert_eq!(text_of(Bytes::from(vec![b'a'; EDIT_FILE_BYTES + 1])), Err(TextRefusal::TooLarge));
    }
}
