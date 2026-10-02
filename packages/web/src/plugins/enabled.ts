import type { ProductState } from '../api/generated/web-api'

/** Whether the user has `plugin` on, by the product state's plugin list. */
export function pluginEnabled(snapshot: ProductState | null, plugin: string): boolean {
  return snapshot?.plugins.some((entry) => entry.id === plugin && entry.enabled) ?? false
}
