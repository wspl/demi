import type { z } from 'zod'
import type { ProductState } from '../api/generated/web-api'

/**
 * The state the plugin `plugin` gave the user's pages, validated against the
 * plugin's own schema, which its page package's types are generated from;
 * none while the snapshot has not arrived or the plugin gives none. A state
 * that does not match is the backend's defect, which this throws for.
 */
export function pluginState<T>(
  snapshot: ProductState | null,
  plugin: string,
  schema: z.ZodType<T>,
): T | null {
  const state = snapshot?.pluginStates[plugin]
  if (state === undefined) {
    return null
  }
  return schema.parse(state)
}
