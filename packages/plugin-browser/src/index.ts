// The `browser` plugin's page (`plugin-pages.md`, `live-view.md` § A
// browser tab in the panel, `preview.md` § What the user sees): its two
// kinds of work panel tab, the agent's browser with the live view and the
// user's browser with the web preview, its panel session, and the types of
// its state, parameters, results and streams, generated from the plugin's
// manifest.
import { definePage } from '@demicodes/plugin-sdk'
import { PLUGIN } from './generated/plugin'
import { browserTabKind } from './live/kind'
import { BrowserTabsController, type BrowserTabsOptions } from './live/tabs'
import { BrowserPanel } from './panel'
import { previewApi, relayDriver } from './preview/api'
import { previewTabKind } from './preview/kind'
import { relay } from './preview/relay'
import { PreviewTabs, type PreviewDriver } from './preview/tabs'
import { browserTabsApi } from './tabs'

export * from './generated/plugin'
export { browserTabsApi } from './tabs'
export { BrowserPanel } from './panel'

/** What the page's panel sessions take: the live view's options, and how previews reach their pages. */
export interface BrowserPageOptions extends BrowserTabsOptions {
  /** The tabs of the user's browser reach their pages through it; the page's relay unless the gallery gives one. */
  previewDriver?: () => PreviewDriver
  /** Why this page cannot show previews; the document's own check unless the gallery gives one. */
  previewUnsupported?: () => string | null
}

/**
 * The page, whose panel sessions take `options`: the page's visibility and
 * the pictures' support, the document's own unless the gallery gives them,
 * and the previews' driver.
 */
export function browserPage(options: BrowserPageOptions = {}) {
  return definePage({
    plugin: PLUGIN,
    kinds: [browserTabKind, previewTabKind],
    panel: (conversation, page) => {
      const plugin = page.plugin.conversation(conversation)
      const session = new AbortController()
      const browser = new BrowserTabsController(
        browserTabsApi(plugin, (panelTab) => page.panel.select(conversation, browserTabKind.kind, panelTab)),
        page.errors,
        options,
      )
      const preview = new PreviewTabs(
        previewApi(
          plugin,
          (data, select) => page.panel.add(conversation, previewTabKind.kind, data, { select }),
          session.signal,
        ),
        options.previewDriver?.() ?? relayDriver(relay()),
        options.previewUnsupported,
      )
      return new BrowserPanel(browser, preview, session)
    },
  })
}

/** The page the registry imports (`plugin-pages.md` § Registration). */
export default browserPage()
