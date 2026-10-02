import { z } from 'zod'
import { PluginCallError, type PluginHost } from '@demicodes/web-ui/plugins/client'
import { liveStreamAt } from '@demicodes/web-ui/transport/live-stream'
import { ApiError, apiRequest, apiUrl, jsonBody, readResponse } from '../api/client'
import type { ProductState } from '../api/generated/web-api'

/**
 * The plugins' host in the product (`plugins.md` § The page): each plugin's
 * state from the product state the sync channel keeps, which drops a plugin
 * the user turned off; its calls over the plugin call routes
 * (`web-api.md` § Plugin calls); and its user streams.
 */
export function productPluginHost(snapshot: () => ProductState | null): PluginHost {
  return {
    state: (plugin) => snapshot()?.pluginStates[plugin],
    async call(plugin, method, params, conversation, options) {
      const scope = conversation === null ? '' : `/conversations/${encodeURIComponent(conversation)}`
      try {
        const response = await apiRequest(`${scope}/plugins/${encodeURIComponent(plugin)}/calls/${encodeURIComponent(method)}`, {
          method: 'POST',
          signal: options?.signal,
          timeoutMs: options?.timeoutMs,
          ...jsonBody(params),
        })
        return await readResponse(response, z.unknown())
      } catch (error) {
        if (error instanceof ApiError) {
          throw new PluginCallError(error.reason ?? error.code ?? `status_${error.status}`, error.message)
        }
        throw error
      }
    },
    stream(name, conversation) {
      const path = `/conversations/${encodeURIComponent(conversation)}/streams/${encodeURIComponent(name)}`
      const url = new URL(apiUrl(path), window.location.href)
      url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
      return liveStreamAt(url.toString())
    },
  }
}

/** Whether the user has `plugin` on, by the product state's plugin list. */
export function pluginEnabled(snapshot: ProductState | null, plugin: string): boolean {
  return snapshot?.plugins.some((entry) => entry.id === plugin && entry.enabled) ?? false
}
