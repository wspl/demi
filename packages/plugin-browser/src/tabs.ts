import { z } from 'zod'
import { BrowserTabsError, type BrowserTabsApi } from '@demicodes/web-ui/browser/tabs'
import { PluginCallError, type ConversationPluginClient } from '@demicodes/web-ui/plugins/client'
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

/**
 * The conversation browser's tab methods and its `browser` user stream over
 * the plugin's client (`live-view.md` § The tab methods). A refusal keeps
 * the plugin's reason, for the tab's content to show.
 */
export function browserTabsApi(plugin: ConversationPluginClient): BrowserTabsApi {
  async function call<T>(method: string, params: object, result: z.ZodType<T>, timeoutMs?: number): Promise<T> {
    try {
      return await plugin.call(method, params, result, { timeoutMs })
    } catch (error) {
      if (error instanceof PluginCallError) {
        throw new BrowserTabsError(error.reason, error.message)
      }
      throw error
    }
  }
  return {
    list: () => call('tabs', {}, browserTabsSchema),
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
