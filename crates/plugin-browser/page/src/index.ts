// The `browser` plugin's page (`packages/plugin-sdk/src/index.ts`): its
// `browser` work panel kind with the live view, its store of each
// conversation's browser, and the types of its state, parameters, results and
// stream, generated from the plugin's manifest.
import { definePage } from '@demicodes/plugin-sdk'
import { PLUGIN } from './generated/plugin'
import { browserTabKind } from './live/kind'
import { browserStore } from './live/store'
import type { BrowserTabsOptions } from './live/tabs'

export * from './generated/plugin'
export { browserTabsApi } from './tabs'

/**
 * The page, whose browsers take `options`: the page's visibility and the
 * pictures' support, the document's own unless the gallery gives them.
 */
export function browserPage(options: BrowserTabsOptions = {}) {
  return definePage({
    plugin: PLUGIN,
    kinds: [browserTabKind(browserStore(options))],
  })
}

/** The page the registry imports (`crates/xtask/src/contracts/registry.rs`). */
export default browserPage()
