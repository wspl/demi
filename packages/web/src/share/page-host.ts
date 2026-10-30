import { createPinia } from 'pinia'
import { PluginCallError, type PageHost } from '@demicodes/web-ui/plugins/page'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { BLOCK_PAGES } from '../plugins/generated/blocks'

/**
 * The plugin pages' host of a shared conversation's page: the pages that draw
 * reply blocks, whose facts the share's pages bring with its texts and whose
 * documents its routes serve for the blocks its text names
 * (`crates/backend-http/src/shares.rs`). A shared page has no account, Host,
 * work panel or plugin calls, so whatever else a page asks of it is refused or
 * empty; its plugins are on, as their blocks' facts came.
 */
export function sharePageHost(token: string): PageHost {
  const none = (what: string): never => {
    throw new PluginCallError('unavailable', `A shared conversation has no ${what}`)
  }
  return {
    pages: () => BLOCK_PAGES,
    blockDocument: (plugin, tag, id) =>
      `/api/shares/${encodeURIComponent(token)}/plugins/${encodeURIComponent(plugin)}/blocks/${encodeURIComponent(tag)}/${encodeURIComponent(id)}`,
    userState: () => undefined,
    enabled: () => true,
    features: () => ({}),
    setFeature: async () => none('settings'),
    followState: () => none('conversation state'),
    call: async () => none('plugin calls'),
    stream: () => none('streams'),
    installed: () => [],
    downloading: () => [],
    hostStarting: () => false,
    files: () => none('files'),
    intents: { open: () => none('work panel'), canOpen: () => false },
    panel: { tabs: () => [], select: () => none('work panel'), canGo: () => false, go: () => {} },
    workspaces: { list: () => [], current: () => null },
    devices: { list: () => [] },
    openSettings: () => none('settings'),
    openLink: async (url) => {
      window.open(await url, '_blank', 'noopener,noreferrer')
    },
    overlays: appOverlayStore,
    pinia: createPinia(),
  }
}
