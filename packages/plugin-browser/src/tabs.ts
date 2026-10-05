import { computed } from 'vue'
import { z } from 'zod'
import { BrowserTabsError, type BrowserTabsApi } from './live/tabs'
import { PluginCallError, type ConversationPlugin } from '@demicodes/plugin-sdk'
import {
  browserTabsSchema,
  type BindTab,
  type NavigateTab,
  type SyncTabs,
  type TabHistory,
} from './generated/plugin'

/**
 * Opening a tab may start the conversation browser, on a Cloud that was
 * stopped: the operation allows itself five minutes (`browser.open`), and the
 * call waits a little longer than that, so the operation's own answer is what
 * ends the wait.
 */
const BIND_TIMEOUT_MS = 310_000

/** A refusal as the tab's content shows it, with the plugin's reason. */
function tabsError(error: PluginCallError): BrowserTabsError {
  return new BrowserTabsError(error.reason, error.message)
}

/**
 * The conversation browser's tab list, its tab methods and its `browser`
 * user stream over the plugin (`live-view.md` § The tab methods), for a
 * panel session, whose effect scope the tab list is followed in.
 */
export function browserTabsApi(plugin: ConversationPlugin): BrowserTabsApi {
  const tabs = plugin.state(browserTabsSchema)
  async function call(method: string, params: object, timeoutMs?: number): Promise<void> {
    try {
      await plugin.call(method, params, z.null(), { timeoutMs })
    } catch (error) {
      throw error instanceof PluginCallError ? tabsError(error) : error
    }
  }
  return {
    tabs: {
      value: tabs.value,
      error: computed(() => {
        const error = tabs.error.value
        return error ? tabsError(error) : null
      }),
    },
    bind: (panelTab) => call('bind', { panelTab } satisfies BindTab, BIND_TIMEOUT_MS),
    sync: () => call('sync', {} satisfies SyncTabs),
    navigate: (tab, url) => call('navigate', { tab, url } satisfies NavigateTab),
    history: (tab, action) => call('history', { tab, action } satisfies TabHistory),
    stream: plugin.stream('browser'),
    installed: () => plugin.installed.value,
  }
}
