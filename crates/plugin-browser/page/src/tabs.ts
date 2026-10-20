import { computed, onScopeDispose } from 'vue'
import { z } from 'zod'
import { BrowserTabsError, asTabsError, type BrowserTabsApi } from './live/tabs'
import { PluginCallError, type ConversationPlugin } from '@demicodes/plugin-sdk'
import {
  browserTabsSchema,
  panelReasonSchema,
  tabBoundSchema,
  tabMovedSchema,
  type BindTab,
  type NavigateTab,
  type StopTab,
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
 * A refused call in the words the page shows for it, never the Host's own error
 * text: its reason's, or for an answer whose reason is not one of the plugin's,
 * its HTTP status. A call no answer came for, or whose answer could not be
 * read, says which.
 */
function tabsError(error: PluginCallError): BrowserTabsError {
  const reason = panelReasonSchema.safeParse(error.reason)
  if (reason.success) {
    return BrowserTabsError.of(reason.data)
  }
  switch (error.answer.type) {
    case 'status':
      return new BrowserTabsError({ status: error.answer.status })
    case 'none':
      return new BrowserTabsError({ answer: 'none' })
    case 'unreadable':
      return new BrowserTabsError({ answer: 'unreadable' })
  }
}

/**
 * The conversation browser's tab list, its tab methods and its `browser` user
 * stream over the plugin, for a conversation's browser on the page, whose
 * effect scope the tab list is followed in. A call still waiting for the
 * backend when the store releases that browser, as when the plugin is turned
 * off, is dropped and never sent (`packages/web-ui/src/plugins/page.ts`).
 */
export function browserTabsApi(plugin: ConversationPlugin, select: (panelTab: string) => void): BrowserTabsApi {
  const tabs = plugin.state(browserTabsSchema)
  const session = new AbortController()
  onScopeDispose(() => session.abort())
  async function call<T>(method: string, params: object, answer: z.ZodType<T>, timeoutMs?: number): Promise<T> {
    try {
      return await plugin.call(method, params, answer, { timeoutMs, signal: session.signal })
    } catch (error) {
      if (session.signal.aborted) {
        // Dropped with the panel: the abort, which nothing reports.
        throw error
      }
      // Anything but a refusal has no code.
      throw error instanceof PluginCallError ? tabsError(error) : asTabsError(error)
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
    select,
    navigate: (tab, url) => move('navigate', { tab, url } satisfies NavigateTab),
    history: (tab, action) => move('history', { tab, action } satisfies TabHistory),
    stop: (tab) => move('stop', { tab } satisfies StopTab),
    stream: plugin.stream('browser'),
    hostStarting: () => plugin.hostStarting.value,
  }
}
