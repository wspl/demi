// The `browser` plugin's page (`plugin-pages.md`, `live-view.md` § A
// browser tab in the panel): its `browser` work panel kind with the live
// view, its panel session, and the types of its state, parameters, results
// and stream, generated from the plugin's manifest.
import { definePage } from '@demicodes/plugin-sdk'
import { PLUGIN } from './generated/plugin'
import { browserTabKind } from './live/kind'
import {
  BrowserTabsController,
  browserTabDataSchema,
  type BrowserTabData,
  type BrowserTabsOptions,
} from './live/tabs'
import { browserTabsApi } from './tabs'

export * from './generated/plugin'
export { browserTabsApi } from './tabs'

/** The `data` of the panel's `browser` tabs that fit the kind; one that does not binds nothing. */
function boundTabs(tabs: readonly unknown[]): BrowserTabData[] {
  const bound: BrowserTabData[] = []
  for (const data of tabs) {
    const parsed = browserTabDataSchema.safeParse(data)
    if (parsed.success) {
      bound.push(parsed.data)
    }
  }
  return bound
}

/**
 * The page, whose panel sessions take `options`: the page's visibility and
 * the pictures' support, the document's own unless the gallery gives them.
 */
export function browserPage(options: BrowserTabsOptions = {}) {
  return definePage({
    plugin: PLUGIN,
    kinds: [browserTabKind],
    panel: (conversation, page) =>
      new BrowserTabsController(
        browserTabsApi(page.plugin.conversation(conversation)),
        {
          bound: () => boundTabs(page.panel.tabs(conversation, browserTabKind.kind)),
          add: (data) => page.panel.add(conversation, browserTabKind.kind, data),
        },
        page.errors.defect,
        options,
      ),
  })
}

/** The page the registry imports (`plugin-pages.md` § Registration). */
export default browserPage()
