/** A timestamp that many milliseconds ago, so fixtures read as "just now" whenever they run. */
export function ago(ms: number): string {
  return new Date(Date.now() - ms).toISOString()
}
