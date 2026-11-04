import { createPinia } from 'pinia'
import { PluginCallError, type PageHost } from '@demicodes/web-ui/plugins/page'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'

/**
 * The plugin pages' host of a page without an account, such as a share's or
 * a view link's: it has no Host, work panel or plugin calls, so whatever a
 * page asks of it beyond `own` is refused, as a `page`'s, or empty; its
 * plugins are on.
 */
export function accountlessPageHost(page: string, own: Pick<PageHost, 'pages' | 'blockDocument'>): PageHost {
  const none = (what: string): never => {
    throw new PluginCallError('unavailable', `${page} has no ${what}`)
  }
  return {
    ...own,
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
