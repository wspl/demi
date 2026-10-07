/**
 * The product's side of the tabs of the user's browser: the plugin's
 * `preview_open` method and `preview` stream for a conversation, and the
 * relay, which serves every tab's frame.
 */
import { PluginCallError, type ConversationPlugin, type PreviewPlace } from '@demicodes/plugin-sdk'
import { previewOpenedSchema, type PreviewOpenInput } from '../generated/plugin'
import { refusalSentence } from '../live/tabs'
import { previewClient } from './client'
import { PreviewConnection } from './connection'
import type { PreviewRelay } from './relay'
import type { PreviewApi, PreviewDriver, PreviewTabData } from './tabs'

/**
 * Opening may wake a stopped Cloud: the call waits as long as opening a tab
 * of the agent's browser does.
 */
const OPEN_TIMEOUT_MS = 310_000

/** The conversation's previews over its plugin, until the session's `signal` aborts. */
export function previewApi(
  plugin: ConversationPlugin,
  add: (data: PreviewTabData, select: boolean) => void,
  signal: AbortSignal,
): PreviewApi {
  return {
    place: plugin.preview,
    connection: new PreviewConnection(plugin.stream('preview')),
    hostStarting: () => plugin.hostStarting.value,
    async open(url: string, place: PreviewPlace) {
      const params: PreviewOpenInput = {
        url,
        scheme: place.scheme,
        domain: place.domain,
        namespace: place.namespace,
        host: place.host,
      }
      try {
        return await plugin.call('preview_open', params, previewOpenedSchema, { timeoutMs: OPEN_TIMEOUT_MS, signal })
      } catch (error) {
        // A refusal shows in the words the page has for its reason, never the Host's own text.
        throw error instanceof PluginCallError ? new Error(refusalSentence(error.reason)) : error
      }
    },
    add,
  }
}

/** The tabs' frames served by the page's relay. */
export function relayDriver(relay: PreviewRelay): PreviewDriver {
  /** The icon each tab shows, which goes when another replaces it or the tab ends. */
  const icons = new Map<string, string>()
  return {
    boots: true,
    register(tab) {
      const unregister = relay.register(tab)
      return () => {
        unregister()
        const icon = icons.get(tab.id)
        if (icon) {
          URL.revokeObjectURL(icon)
          icons.delete(tab.id)
        }
      }
    },
    async boot(_tab, place, opened, navigation) {
      relay.know(place, { [opened.label]: opened.environment })
      return relay.bootAddress(opened, navigation)
    },
    command: (tab, command) => relay.command(tab, command),
    async icon(tab, place, page) {
      const environment = relay.topEnvironment(tab)
      if (!environment) {
        return null
      }
      // The icon loads as the page's own image would, through the stream.
      const exchange = tab.connection.request(
        place,
        environment,
        {
          url: page.icon,
          method: 'GET',
          headers: [],
          mode: 'no-cors',
          destination: 'image',
          credentials: 'include',
          referrer: page.url,
          referrerPolicy: '',
          keepalive: false,
          initiator: environment,
          user: false,
        },
        previewClient(),
        null,
      )
      const head = await exchange.head
      if (head.status !== 200) {
        exchange.cancel()
        return null
      }
      const chunks: Uint8Array<ArrayBuffer>[] = []
      for (let chunk = await exchange.pull(); chunk; chunk = await exchange.pull()) {
        chunks.push(chunk)
      }
      const type = head.headers.find((header) => header.name.toLowerCase() === 'content-type')?.value ?? ''
      if (!type.startsWith('image/')) {
        return null
      }
      const icon = URL.createObjectURL(new Blob(chunks, { type }))
      const before = icons.get(tab.id)
      if (before) {
        URL.revokeObjectURL(before)
      }
      icons.set(tab.id, icon)
      return icon
    },
  }
}
