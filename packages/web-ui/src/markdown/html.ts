/**
 * Text put into rendered HTML, as text: the four characters that would
 * otherwise start a tag, an entity, or end an attribute value. Every renderer
 * here escapes the same way, so a fragment is safe wherever it lands.
 */
export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

/**
 * Whether `html` is a line break, `<br>`, `<br/>` or `<br />`: the one tag a
 * reply's Markdown keeps, since a table's cell has no other way to break its
 * line, and GitHub's rendering, which models write for, breaks it there.
 */
export function isLineBreak(html: string): boolean {
  return /^<br\s*\/?>$/i.test(html.trim())
}
