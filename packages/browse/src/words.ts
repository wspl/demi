// How a command line written as one string, such as the action of
// `timeline 'click role=button[name="Reload"]'`, splits into words: at
// spaces, except inside quotes or a locator's brackets, so
// `role=button[name="Reload Page"]` stays one word. Quotes inside brackets
// stay, since the locator needs them; quotes around a whole word go.
export function splitWords(line: string): string[] {
  const words: string[] = []
  let word = ''
  let started = false
  let quote: '"' | '\'' | null = null
  let depth = 0
  for (const char of line) {
    if (quote) {
      if (char === quote) {
        quote = null
        if (depth > 0) {
          word += char
        }
      } else {
        word += char
      }
      continue
    }
    if ((char === '"' || char === '\'')) {
      quote = char
      started = true
      if (depth > 0) {
        word += char
      }
      continue
    }
    if (char === '[') {
      depth += 1
    } else if (char === ']' && depth > 0) {
      depth -= 1
    }
    if (/\s/.test(char) && depth === 0) {
      if (started) {
        words.push(word)
      }
      word = ''
      started = false
      continue
    }
    word += char
    started = true
  }
  if (quote) {
    throw new Error(`unclosed ${quote} in ${line}`)
  }
  if (started) {
    words.push(word)
  }
  return words
}
