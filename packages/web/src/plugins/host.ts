import { z } from 'zod'
import { PluginCallError, type PluginHost } from '@demicodes/web-ui/plugins/client'
import { userStreamAt } from '@demicodes/web-ui/transport/user-stream'
import { ApiError, apiRequest, apiUrl, jsonBody, readResponse } from '../api/client'
import type { ProductState } from '../api/generated/web-api'
import { executionFor } from '../targets/execution'
import { packageInstalls } from '../state/installs'

/**
 * The plugins' host in the product (`plugins.md` § The page): each plugin's
 * state from the product state the sync channel keeps, which drops a plugin
 * the user turned off; its calls over the plugin call routes
 * (`web-api.md` § Plugin calls); its user streams; and the installs of its
 * packages on a conversation's main Host, from that device's in the product
 * state.
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
      return userStreamAt(url.toString())
    },
    installs(plugin, conversation) {
      const state = snapshot()
      const summary = state?.conversations.find((entry) => entry.id === conversation)
      if (!state || !summary) {
        return []
      }
      const packages = state.plugins.find((entry) => entry.id === plugin)?.packages ?? []
      return packageInstalls(state, executionFor(summary).deviceId, packages)
    },
  }
}

/** Whether the user has `plugin` on, by the product state's plugin list. */
export function pluginEnabled(snapshot: ProductState | null, plugin: string): boolean {
  return snapshot?.plugins.some((entry) => entry.id === plugin && entry.enabled) ?? false
}
