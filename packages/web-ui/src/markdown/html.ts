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
