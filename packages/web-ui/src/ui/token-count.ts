export type TokenUnit = 'K' | 'M'
const SCALE: Record<TokenUnit, number> = { K: 1_000, M: 1_000_000 }

export function displayTokenCount(
  tokens: number | null,
  unit: TokenUnit,
): string {
  if (tokens === null) {
    return ''
  }
  const count = tokens / SCALE[unit]
  return String(Number(count.toFixed(unit === 'K' ? 3 : 6)))
}

/** UI unit conversion accepts separators and emits whole, safely representable tokens. */
export function parseTokenCount(value: string, unit: TokenUnit): number | null {
  const text = value.replace(/[,\s]/g, '')
  if (!text) {
    return null
  }
  const count = Number(text)
  const tokens = Math.round(count * SCALE[unit])
  return count >= 0 && Number.isSafeInteger(tokens) ? tokens : null
}
