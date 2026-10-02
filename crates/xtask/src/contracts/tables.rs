//! The constant tables the page shares with the Rust side, with their
//! lookups (`contracts.md` § Logic the web app and backend share): the
//! file-type table the page chooses a viewer by, and the file types a model
//! reads. The values come from the Rust constants; the lookups do what
//! core's lookups do, and the page's tests check them with core's cases.

use std::fmt::Write as _;

use demi_shared_types::{ATTACHMENT_FILE_EXTENSIONS, PREVIEW_TYPES, VIDEO_FILE_EXTENSIONS};

use super::zod::push_doc;

/// The module's source; it imports the tables' types from `contracts`.
pub fn module(header: &str) -> String {
    let mut source = String::from(header);
    source.push_str("import type { FileExtension, PreviewType } from \"./contracts\"\n");

    source.push('\n');
    push_doc(
        &mut source,
        Some(
            "The file-type table (`file-previews.md` § Choosing a view): the media type\n\
             the product knows a file by, from its extension, and whether the page shows\n\
             it in place. The backend serves files by the same table.",
        ),
        0,
    );
    let table = serde_json::to_string_pretty(&PREVIEW_TYPES).expect("the table serializes");
    writeln!(
        source,
        "export const PREVIEW_TYPES: readonly PreviewType[] = {table}"
    )
    .expect("writing to a string");
    source.push_str(LOOKUPS);

    for (name, description, extensions) in [
        (
            "ATTACHMENT_FILE_EXTENSIONS",
            "The image and document types a model known to read attachments accepts.",
            &ATTACHMENT_FILE_EXTENSIONS[..],
        ),
        (
            "VIDEO_FILE_EXTENSIONS",
            "The video types, which only a model known to read video accepts.",
            &VIDEO_FILE_EXTENSIONS[..],
        ),
    ] {
        let extensions = serde_json::to_string(extensions).expect("the extensions serialize");
        source.push('\n');
        push_doc(&mut source, Some(description), 0);
        writeln!(
            source,
            "export const {name}: readonly FileExtension[] = {extensions}"
        )
        .expect("writing to a string");
    }

    source.push('\n');
    push_doc(
        &mut source,
        Some(
            "The largest message a page sends on any of its WebSockets (`web-api.md` § Request bodies).",
        ),
        0,
    );
    writeln!(
        source,
        "export const MAX_PAGE_MESSAGE_BYTES = {}",
        count(demi_web_api_protocol::MAX_PAGE_MESSAGE_BYTES)
    )
    .expect("writing to a string");

    source
}

/// A size as the integer the page reads.
fn count(bytes: usize) -> u64 {
    u64::try_from(bytes).expect("a size fits in 64 bits")
}

/// core's `preview_media_type` and `shows_in_place`, over the table.
const LOOKUPS: &str = r#"
/**
 * The media type the product knows the file at `path` by, from its extension
 * whatever its ASCII case, or null for a file the table does not name. Either
 * separator ends a directory, since a paired Windows device names its paths
 * with `\`, and a name that starts with its only dot, such as `.png`, has no
 * extension.
 */
export function previewMediaType(path: string): string | null {
  const name = path.slice(Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\")) + 1)
  const dot = name.lastIndexOf(".")
  if (dot <= 0) {
    return null
  }
  const extension = name.slice(dot + 1).replace(/[A-Z]/g, (letter) => letter.toLowerCase())
  return PREVIEW_TYPES.find((entry) => entry.extensions.includes(extension))?.mediaType ?? null
}

/** Whether the page shows `mediaType` in place instead of downloading it. */
export function showsInPlace(mediaType: string): boolean {
  return PREVIEW_TYPES.some((entry) => entry.inPlace && entry.mediaType === mediaType)
}
"#;
