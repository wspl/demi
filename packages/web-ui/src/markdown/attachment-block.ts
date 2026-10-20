/**
 * An attachment block: a line whose whole content, after the Markdown that
 * places it (a list item's marker, a quote's `>`), is the tag `<attachment
 * id="a3" />`, the space before `/>` optional and nothing else varying. The
 * backend follows the same rule to know which attachments a share serves
 * (`named_attachments` in `crates/backend-database/src/shares.rs`); both are
 * held to the cases of
 * `crates/shared-types/tests/shared-types/fixtures/attachment-blocks.json`.
 */
const TAG = String.raw`<attachment id="(a[1-9]\d*)" ?\/>[ \t]*`

/** The tag starting a block, up to three spaces in, with its line's end. */
const AT_START = new RegExp(String.raw`^ {0,3}${TAG}(?:\n|$)`)

/** A later line of the text that is the tag, up to three spaces in. */
const A_LATER_LINE = new RegExp(String.raw`\n {0,3}${TAG}(?:\n|$)`)

/**
 * The attachment block that starts `src`, its whole line and the number it
 * names; null when none does.
 */
export function attachmentBlockAt(src: string): { raw: string; id: string } | null {
  const found = AT_START.exec(src)
  return found ? { raw: found[0], id: found[1]! } : null
}

/**
 * Where the first line of `src` after its first that is an attachment block
 * starts, which ends the paragraph before it; undefined when none is. The
 * reader asks it of a paragraph's text less its first character, so a line
 * counts only after a line break, never right after the words before it.
 */
export function attachmentBlockStart(src: string): number | undefined {
  const found = A_LATER_LINE.exec(src)
  return found ? found.index + 1 : undefined
}
