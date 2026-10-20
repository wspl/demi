/**
 * The `browser` page's store (`packages/web-ui/src/plugins/stores.ts`): each
 * conversation's browser on the page (`./tabs.ts`), kept while the plugin is
 * on, by conversation.
 *
 * A browser tab's content watches its conversation's browser while it is
 * mounted, and a tab's title or icon reaches it without watching. One no
 * content watches is kept, with its view's connection and each tab's last
 * picture, so a return to the conversation shows them at once; past the
 * limit, the least recently watched is released.
 */
import { effectScope, onScopeDispose } from 'vue'
import { KeptSessions, definePluginStore, type KeptSession, type PageContext, type PluginStore } from '@demicodes/plugin-sdk'
import { browserTabsApi } from '../tabs'
import { BrowserTabsController, type BrowserTabsOptions } from './tabs'

/** The kind's name, which the page's tabs and the plugin's panel tabs carry. */
export const BROWSER_TAB_KIND = 'browser'

/** One conversation's browser as the store keeps it. */
export interface KeptBrowser extends KeptSession {
  readonly tabs: BrowserTabsController
}

export type BrowserStore = PluginStore<KeptSessions<KeptBrowser>>

/** The store of a page whose browsers take `options`. */
export function browserStore(options: BrowserTabsOptions): BrowserStore {
  return definePluginStore('browsers', (page) => {
    const browsers = new KeptSessions((conversation) => keptBrowser(page, conversation, options))
    // The store ends as its plugin is turned off.
    onScopeDispose(() => browsers.dispose())
    return browsers
  })
}

function keptBrowser(page: PageContext, conversation: string, options: BrowserTabsOptions): KeptBrowser {
  // Its own scope, which ends when the store releases it rather than with whatever made it.
  const scope = effectScope(true)
  const tabs = scope.run(() => new BrowserTabsController(
    browserTabsApi(
      page.plugin.conversation(conversation),
      (panelTab) => page.panel.select(conversation, BROWSER_TAB_KIND, panelTab),
    ),
    page.errors,
    options,
  ))!
  return {
    tabs,
    // What its view watches follows its contents' show and hide (`BrowserTabsController.show`).
    watched: () => {},
    dispose: () => {
      tabs.dispose()
      scope.stop()
    },
  }
}
