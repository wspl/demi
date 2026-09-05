/**
 * Binary resolution and the home directory are device facts (`runner.md`
 * § Responsibilities): a raw spawn whose env names no `PATH` / `HOME` gets
 * this device's own and nothing else of it — the spawner (a provider's CLI
 * transport) owns the rest of that environment. Jobs run in the device
 * environment as a whole (`serve/jobs.ts`).
 */
export function deviceFallback(env: Record<string, string>, device: Record<string, string>): Record<string, string> {
  const merged = { ...env }
  for (const key of ['PATH', 'HOME']) {
    if (!(key in merged) && device[key] !== undefined) merged[key] = device[key]
  }
  return merged
}
