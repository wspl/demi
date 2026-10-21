import { decodeHTMLStrict } from 'entities'

/**
 * The key a reply keeps a picture's copy by: the target of the image as the
 * backend reads it (`crates/agent-transcript/src/reply_markdown.rs`).
 *
 * The page's Markdown reader leaves the character references of a target as
 * they were written (`out/a&amp;b.png`), where the backend's reader resolves
 * them (`out/a&b.png`); every other part of a target they read alike, which
 * `crates/agent-transcript/tests/agent-transcript/fixtures/picture-targets.json`
 * holds both to. So the key is the target with its character references
 * resolved, as CommonMark has them (each ends with a semicolon), and the copy
 * of a picture is found by it whether or not the page knows the file's place.
 */
export function pictureKey(target: string): string {
  return target.includes('&') ? decodeHTMLStrict(target) : target
}
