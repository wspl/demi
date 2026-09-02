/**
 * Binary resolution and the home directory are device facts (`runner.md`
 * § Responsibilities): a spawn or job whose env names no `PATH` / `HOME`
 * gets this device's own.
 */
export function deviceFallback(env: Record<string, string>, device: Record<string, string>): Record<string, string> {
  const merged = { ...env }
  for (const key of ['PATH', 'HOME']) {
    if (!(key in merged) && device[key] !== undefined) merged[key] = device[key]
  }
  return merged
}
