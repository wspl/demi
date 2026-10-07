import { computed } from 'vue'
import { z } from 'zod'
import { BrowserTabsError, refusalSentence, type BrowserTabsApi } from './live/tabs'
import { PluginCallError, type ConversationPlugin } from '@demicodes/plugin-sdk'
import {
  browserTabsSchema,
  tabBoundSchema,
  tabMovedSchema,
  type BindTab,
  type NavigateTab,
  type StopTab,
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

/**
 * A refusal with the plugin's reason, in the words the page shows for it,
 * never the Host's own error text (`live-view.md` § A browser tab in the
 * panel).
 */
function tabsError(error: PluginCallError): BrowserTabsError {
  return new BrowserTabsError(error.reason, refusalSentence(error.reason))
}

/**
 * The conversation browser's tab list, its tab methods and its `browser`
 * user stream over the plugin (`live-view.md` § The tab methods), for a
 * panel session, whose effect scope the tab list is followed in.
 */
export function browserTabsApi(plugin: ConversationPlugin, select: (panelTab: string) => void): BrowserTabsApi {
  const tabs = plugin.state(browserTabsSchema)
  async function call<T>(method: string, params: object, answer: z.ZodType<T>, timeoutMs?: number): Promise<T> {
    try {
      return await plugin.call(method, params, answer, { timeoutMs })
    } catch (error) {
      throw error instanceof PluginCallError ? tabsError(error) : error
    }
  }
  /** A navigation's answer: the number of the last tab list before it started. */
  async function move(method: string, params: object): Promise<number> {
    const moved = await call(method, params, tabMovedSchema)
    return moved.list
  }
  return {
    tabs: {
      value: tabs.value,
      error: computed(() => {
        const error = tabs.error.value
        return error ? tabsError(error) : null
      }),
    },
    bind: async (panelTab) => {
      const bound = await call('bind', { panelTab } satisfies BindTab, tabBoundSchema, BIND_TIMEOUT_MS)
      return bound.tab
    },
    sync: async () => {
      await call('sync', {} satisfies SyncTabs, z.null())
    },
    select,
    navigate: (tab, url) => move('navigate', { tab, url } satisfies NavigateTab),
    history: (tab, action) => move('history', { tab, action } satisfies TabHistory),
    stop: (tab) => move('stop', { tab } satisfies StopTab),
    stream: plugin.stream('browser'),
    installed: () => plugin.installed.value,
  }
}
