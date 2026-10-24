import type { PageState } from '../state/lists'

/** Whether the user has `plugin` on, by the product state's plugin list. */
export function pluginEnabled(snapshot: PageState | null, plugin: string): boolean {
  return snapshot?.plugins?.some((entry) => entry.id === plugin && entry.enabled) ?? false
}
