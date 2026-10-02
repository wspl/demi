import { computed } from 'vue'
import { z } from 'zod'
import { BrowserTabsError, type BrowserTabsApi } from './live/tabs'
import { PluginCallError, type ConversationPlugin } from '@demicodes/plugin-sdk'
import {
  browserTabsSchema,
  openedTabSchema,
  type CloseTab,
  type NavigateTab,
  type OpenTab,
  type TabHistory,
} from './generated/plugin'

/**
 * Opening a tab may start the conversation browser, on a Cloud that was
 * stopped: the operation allows itself five minutes (`browser.open`), and the
 * call waits a little longer than that, so the operation's own answer is what
 * ends the wait.
 */
const OPEN_TIMEOUT_MS = 310_000

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
  async function call<T>(method: string, params: object, result: z.ZodType<T>, timeoutMs?: number): Promise<T> {
    try {
      return await plugin.call(method, params, result, { timeoutMs })
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
      read: tabs.read,
    },
    open: async (url) => (await call('open', { url } satisfies OpenTab, openedTabSchema, OPEN_TIMEOUT_MS)).tab,
    close: async (tab) => {
      await call('close', { tab } satisfies CloseTab, z.null())
    },
    navigate: async (tab, url) => {
      await call('navigate', { tab, url } satisfies NavigateTab, z.null())
    },
    history: async (tab, action) => {
      await call('history', { tab, action } satisfies TabHistory, z.null())
    },
    stream: plugin.stream('browser'),
    installs: () => plugin.installs.value,
  }
}
