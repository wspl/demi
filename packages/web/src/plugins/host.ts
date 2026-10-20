import { getActivePinia } from 'pinia'
import { watch } from 'vue'
import { z } from 'zod'
import type { PageHost } from '@demicodes/web-ui/plugins/page'
import { disposePluginStores } from '@demicodes/web-ui/plugins/stores'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { userStreamAt } from '@demicodes/web-ui/transport/user-stream'
import { apiRequest, apiUrl, jsonBody, readResponse } from '../api/client'
import { conversationFiles } from '../conversation/files'
import { directRoute } from '../direct'
import { directStream } from '../direct/operations'
import { useWorkPanel } from '../conversation/work'
import { useProduct } from '../state/product'
import { packageInstalled } from '../state/installed'
import { useResources } from '../state/resources'
import { useSettingsAddress } from '../settings/address'
import { executionFor } from '../targets/execution'
import { pluginEnabled } from './enabled'
import { callFailure } from './errors'
import { PLUGIN_PAGES } from './generated/pages'
import { conversationStates } from './states'

/**
 * The plugin pages' host in the product. It supplies:
 *
 * - Each plugin's user state, from the product state the sync channel keeps,
 *   which drops a plugin the user turned off.
 * - Its conversation states, by revision.
 * - Its calls, over the plugin call routes. These calls and the states' reads
 *   wait in the HTTP client while the backend cannot be reached.
 * - Its user streams.
 * - What a conversation's primary Host holds of its packages, from that
 *   device's in the product state, and whether that Host is a Cloud that does
 *   not run, from the Cloud's state there.
 * - The shell's own services: the conversations' files, intents and panels, and
 *   the settings dialog.
 * - The app's Pinia, which holds the plugins' stores; a plugin the user turns
 *   off has its stores disposed then, and only its own.
 */
/**
 * Opens a tab now, in the click that asked for it, and sends it to `url`
 * once that arrives; a URL that never comes closes it.
 */
async function openLink(url: string | Promise<string>): Promise<void> {
  // Without `noopener` the tab is returned to point it; its opener is
  // dropped before it loads anything.
  const tab = window.open('', '_blank')
  if (tab) {
    tab.opener = null
  }
  try {
    const target = await url
    if (tab) {
      tab.location.href = target
    } else {
      // A browser that refused even the blank tab may still allow a
      // navigation in a new one.
      window.open(target, '_blank', 'noopener,noreferrer')
    }
  } catch (error) {
    tab?.close()
    throw error
  }
}

export function productPageHost(): PageHost {
  const pinia = getActivePinia()
  if (!pinia) {
    throw new Error('the plugin pages\' host is made in the app, which has its Pinia')
  }
  const product = useProduct()
  const work = useWorkPanel()
  const resources = useResources()
  const settings = useSettingsAddress()
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
  // Each plugin is watched on its own, so a switch of one leaves the others' stores alone.
  for (const page of PLUGIN_PAGES) {
    watch(
      () => pluginEnabled(product.snapshot, page.plugin),
      (enabled) => {
        if (!enabled) {
          disposePluginStores(pinia, page.plugin)
        }
      },
    )
  }
  return {
    userState: (plugin) => product.snapshot?.pluginStates[plugin],
    followState: states.follow,
    async call(plugin, method, params, conversation, options) {
      const scope = conversation === null ? '' : `/conversations/${encodeURIComponent(conversation)}`
      const path = `${scope}/plugins/${encodeURIComponent(plugin)}/calls/${encodeURIComponent(method)}`
      try {
        // A call that cannot reach the backend waits for it in the HTTP client and goes then
        // (`packages/web-ui/src/plugins/page.ts`).
        const response = await apiRequest(path, {
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
      // Over the direct channel while the conversation's device has one.
      return directStream(() => directRoute(conversation), name, userStreamAt(url.toString()))
    },
    installed(plugin, conversation) {
      const host = primaryHost(plugin, conversation)
      return host ? packageInstalled(product.snapshot, host.deviceId, host.packages) : []
    },
    hostStarting(conversation) {
      const state = product.snapshot
      const summary = state?.conversations.find((entry) => entry.id === conversation)
      return !!state && !!summary && executionFor(summary).kind === 'cloud' && state.cloud.state !== 'running'
    },
    files: conversationFiles,
    intents: {
      open: (conversation, request) => work.openIn(conversation, request),
      canOpen: (intent) => work.canOpen(intent),
    },
    panel: {
      tabs: (conversation, kind) =>
        work.stateFor(conversation).panel.tabs.filter((tab) => tab.kind === kind).map((tab) => tab.data),
      select: (conversation, _kind, id) => {
        work.select(conversation, id)
        work.setOpen(work.stateFor(conversation), true)
      },
    },
    openSettings: (section) => void settings.open(section),
    openLink,
    overlays: appOverlayStore,
    pinia,
  }
}
