const THOUSAND = 1_000
const MILLION = 1_000_000

/**
 * A token count as the UI shows it: `1.2M`, `128K`, `640`, and an em dash when
 * there is no count at all. One decimal at most, and none when it would be a
 * zero, so a context window reads `200K` and a live count reads `12.3K`.
 */
export function formatTokens(tokens: number | null): string {
  if (tokens === null)
    return '—'
  if (tokens >= MILLION)
    return `${scaled(tokens / MILLION)}M`
  if (tokens >= THOUSAND)
    return `${scaled(tokens / THOUSAND)}K`
  return String(tokens)
}

function scaled(value: number): string {
  const text = value.toFixed(1)
  return text.endsWith('.0') ? text.slice(0, -2) : text
}
