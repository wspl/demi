/**
 * A credential block: a line whose whole content, after the Markdown that
 * places it, is the tag `<credential name="Deploy key" />`, with
 * `field="token"` after the name for one field of several, the space before
 * `/>` optional, as an attachment block is its tag's (`attachment-block.ts`).
 * The name and the field are escaped as XML escapes an attribute: `&quot;`,
 * `&amp;`, `&lt;`, `&gt;` and `&apos;` stand for their characters.
 */
export const CREDENTIAL_TAG = String.raw`<credential name="([^"<\n]+)"(?: field="([^"<\n]+)")? ?\/>[ \t]*`

/** The tag starting a block, up to three spaces in, with its line's end. */
const AT_START = new RegExp(String.raw`^ {0,3}${CREDENTIAL_TAG}(?:\n|$)`)

/** A later line of the text that is the tag, up to three spaces in. */
const A_LATER_LINE = new RegExp(String.raw`\n {0,3}${CREDENTIAL_TAG}(?:\n|$)`)

const ENTITIES: Readonly<Record<string, string>> = { quot: '"', amp: '&', lt: '<', gt: '>', apos: '\'' }

/** An attribute's text as written, its entities read as their characters. */
function unescaped(text: string): string {
  return text.replace(/&(quot|amp|lt|gt|apos);/g, (_, entity: string) => ENTITIES[entity]!)
}

/**
 * The credential block that starts `src`, its whole line, the credential's
 * name and the field it names, null for every field; null when none does.
 */
export function credentialBlockAt(src: string): { raw: string; name: string; field: string | null } | null {
  const found = AT_START.exec(src)
  return found ? { raw: found[0], name: unescaped(found[1]!), field: found[2] === undefined ? null : unescaped(found[2]) } : null
}

/**
 * Where the first line of `src` after its first that is a credential block
 * starts, which ends the paragraph before it; undefined when none is.
 */
export function credentialBlockStart(src: string): number | undefined {
  const found = A_LATER_LINE.exec(src)
  return found ? found.index + 1 : undefined
}
