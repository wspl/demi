/** Single-quotes a value for safe use as one shell word. */
export function shellQuote(value: string): string {
  return `'${value.replaceAll("'", `'\\''`)}'`
}

/** Picks a heredoc delimiter that does not collide with any line already in `body`. */
export function heredocDelimiter(body: string): string {
  let delimiter = 'DEMI_STDIN'
  const lines = new Set(body.split('\n'))
  while (lines.has(delimiter)) delimiter += '_X'
  return delimiter
}
