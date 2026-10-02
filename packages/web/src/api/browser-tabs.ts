import { BrowserTabsError, type BrowserTabsApi } from '@demicodes/web-ui/browser/tabs'
import { liveStreamAt } from '@demicodes/web-ui/transport/live-stream'
import {
  browserTabsSchema,
  openedTabSchema,
  type CloseTab,
  type NavigateTab,
  type OpenTab,
  type TabHistory,
} from '@demicodes/plugin-browser'
import { z } from 'zod'
import { ApiError, apiRequest, apiUrl, jsonBody, readResponse } from './client'

/**
 * Opening a tab may start the conversation browser, on a Cloud that was
 * stopped: the operation allows itself five minutes (`browser.open`), and the
 * request waits a little longer than that, so the operation's own answer is
 * what ends the wait.
 */
const OPEN_TIMEOUT_MS = 310_000

/** A method that answers nothing answers `null`. */
const nothingSchema = z.null()

/**
 * The conversation browser's tab methods, called through the plugin call
 * route, and its user stream (`live-view.md` § The tab methods). A refusal
 * keeps the backend's own word for it, the plugin's reason first, for the
 * tab's content to show.
 */
export function browserTabsApi(conversationId: string): BrowserTabsApi {
  const base = `/conversations/${encodeURIComponent(conversationId)}`
  const stream = new URL(apiUrl(`${base}/streams/browser`), window.location.href)
  stream.protocol = stream.protocol === 'https:' ? 'wss:' : 'ws:'
  async function call<T>(
    method: string,
    params: object,
    schema: z.ZodType<T>,
    timeoutMs?: number,
  ): Promise<T> {
    try {
      const response = await apiRequest(`${base}/plugins/browser/calls/${method}`, {
        method: 'POST',
        timeoutMs,
        ...jsonBody(params),
      })
      return await readResponse(response, schema)
    } catch (error) {
      if (error instanceof ApiError) {
        throw new BrowserTabsError(error.reason ?? error.code, error.message)
      }
      throw error
    }
  }
  return {
    list: () => call('tabs', {}, browserTabsSchema),
    open: async (url) => (await call('open', { url } satisfies OpenTab, openedTabSchema, OPEN_TIMEOUT_MS)).tab,
    close: async (tab) => {
      await call('close', { tab } satisfies CloseTab, nothingSchema)
    },
    navigate: async (tab, url) => {
      await call('navigate', { tab, url } satisfies NavigateTab, nothingSchema)
    },
    history: async (tab, action) => {
      await call('history', { tab, action } satisfies TabHistory, nothingSchema)
    },
    stream: liveStreamAt(stream.toString()),
  }
}
