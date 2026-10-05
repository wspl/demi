import { z } from 'zod'
import type { PageHost } from '@demicodes/web-ui/plugins/page'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { userStreamAt } from '@demicodes/web-ui/transport/user-stream'
import { apiRequest, apiUrl, jsonBody, readResponse } from '../api/client'
import { conversationFiles } from '../conversation/files'
import { useWorkPanel } from '../conversation/work'
import { useProduct } from '../state/product'
import { packageInstalled, packageInstalls } from '../state/installs'
import { useResources } from '../state/resources'
import { executionFor } from '../targets/execution'
import { callFailure } from './errors'
import { conversationStates } from './states'

/**
 * The plugin pages' host in the product (`plugins.md` § The page): each
 * plugin's user state from the product state the sync channel keeps, which
 * drops a plugin the user turned off; its conversation states by revision;
 * its calls over the plugin call routes (`web-api.md` § Plugin calls); its
 * user streams; the installs of its packages on a conversation's primary Host,
 * and what that Host holds of them, from that device's in the product state;
 * and the shell's own services: the conversations' files, intents and panels,
 * and the settings dialog.
 */
export function productPageHost(): PageHost {
  const product = useProduct()
  const work = useWorkPanel()
  const resources = useResources()
  const states = conversationStates(() => product.snapshot)
  /** `conversation`'s primary Host and `plugin`'s packages, once the product state names both. */
  function primaryHost(plugin: string, conversation: string) {
    const state = product.snapshot
    const summary = state?.conversations.find((entry) => entry.id === conversation)
    if (!state || !summary) {
      return null
    }
    const packages = state.plugins.find((entry) => entry.id === plugin)?.packages ?? []
    return { deviceId: executionFor(summary).deviceId, packages }
  }
  return {
    userState: (plugin) => product.snapshot?.pluginStates[plugin],
    followState: states.follow,
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
        throw callFailure(error)
      }
    },
    stream(name, conversation) {
      const path = `/conversations/${encodeURIComponent(conversation)}/streams/${encodeURIComponent(name)}`
      const url = new URL(apiUrl(path), window.location.href)
      url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
      return userStreamAt(url.toString())
    },
    installs(plugin, conversation) {
      const host = primaryHost(plugin, conversation)
      return host ? packageInstalls(product.snapshot, host.deviceId, host.packages) : []
    },
    installed(plugin, conversation) {
      const host = primaryHost(plugin, conversation)
      return host ? packageInstalled(product.snapshot, host.deviceId, host.packages) : []
    },
    files: conversationFiles,
    intents: {
      open: (conversation, request) => work.openIn(conversation, request),
      canOpen: (intent) => work.canOpen(intent),
    },
    panel: {
      tabs: (conversation, kind) =>
        work.stateFor(conversation).panel.tabs.filter((tab) => tab.kind === kind).map((tab) => tab.data),
      add: (conversation, kind, data, options = { select: false }) => void work.add(conversation, kind, data, options),
    },
    openSettings: (section) => resources.openSettings(section),
    overlays: appOverlayStore,
  }
}
