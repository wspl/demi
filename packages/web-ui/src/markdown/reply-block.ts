/**
 * A plugin's reply block: a line whose whole content, after the Markdown that
 * places it, is `<tag id="w1" />` for a tag a plugin's page owns
 * (`plugins/page.ts` § ReplyBlock), as an attachment block is its tag's
 * (`attachment-block.ts`). The backend follows the same rule to know what a
 * share serves (`named_blocks` in `crates/backend-database/src/shares.rs`).
 */
const LINE = String.raw`<([a-z][a-z0-9-]*) id="([A-Za-z0-9_-]+)" ?\/>[ \t]*`

/** The line starting a block, up to three spaces in, with its line's end. */
const AT_START = new RegExp(String.raw`^ {0,3}${LINE}(?:\n|$)`)

/** A later line of the text that is such a line, up to three spaces in. */
const A_LATER_LINE = new RegExp(String.raw`\n {0,3}${LINE}(?:\n|$)`, 'g')

/** The attribute of a block's place naming its tag, and the one naming its id. */
export const REPLY_BLOCK_ATTRIBUTE = 'data-reply-block'
export const REPLY_BLOCK_ID_ATTRIBUTE = 'data-reply-block-id'

/** The reply block of one of `tags` that starts `src`, its whole line, tag and id; null when none does. */
export function replyBlockAt(src: string, tags: readonly string[]): { raw: string; tag: string; id: string } | null {
  const found = AT_START.exec(src)
  return found && tags.includes(found[1]!) ? { raw: found[0], tag: found[1]!, id: found[2]! } : null
}

/**
 * Where the first line of `src` after its first that is a reply block of one
 * of `tags` starts; undefined when none is.
 */
export function replyBlockStart(src: string, tags: readonly string[]): number | undefined {
  for (const found of src.matchAll(A_LATER_LINE)) {
    if (tags.includes(found[1]!))
      return found.index + 1
  }
  return undefined
}
