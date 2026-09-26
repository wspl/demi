import { BrowserTabsError, type BrowserTabsApi } from '@demicodes/web-ui/browser/tabs'
import { liveStreamAt } from '@demicodes/web-ui/transport/live-stream'
import { ApiError, apiRequest, apiUrl, jsonBody, readResponse } from './client'
import {
  browserTabSchema,
  browserTabsSchema,
  type NavigateTab,
  type OpenTab,
  type TabHistory,
} from './generated/web-api'

/**
 * Opening a tab may start the browser, on a Cloud that was stopped: the
 * operation allows itself five minutes (`browser.open`), and the request
 * waits a little longer than that, so the operation's own answer is what ends
 * the wait.
 */
const OPEN_TIMEOUT_MS = 310_000

/** A refusal keeps the backend's own code and message for the tab's content to show. */
async function request(path: string, init?: Parameters<typeof apiRequest>[1]): Promise<Response> {
  try {
    return await apiRequest(path, init)
  } catch (error) {
    if (error instanceof ApiError) {
      throw new BrowserTabsError(error.code, error.message)
    }
    throw error
  }
}

/** The conversation browser's tab routes and user stream (`web-api.md` § Conversation browser tabs). */
export function browserTabsApi(conversationId: string): BrowserTabsApi {
  const base = `/conversations/${encodeURIComponent(conversationId)}`
  const tabs = `${base}/browser/tabs`
  const stream = new URL(apiUrl(`${base}/streams/browser`), window.location.href)
  stream.protocol = stream.protocol === 'https:' ? 'wss:' : 'ws:'
  return {
    list: async () => readResponse(await request(tabs), browserTabsSchema),
    open: async (url) => readResponse(
      await request(tabs, { method: 'POST', timeoutMs: OPEN_TIMEOUT_MS, ...jsonBody({ url } satisfies OpenTab) }),
      browserTabSchema,
    ),
    close: async (tab) => {
      await request(`${tabs}/${encodeURIComponent(tab)}`, { method: 'DELETE' })
    },
    navigate: async (tab, url) => {
      await request(`${tabs}/${encodeURIComponent(tab)}/navigate`, { method: 'POST', ...jsonBody({ url } satisfies NavigateTab) })
    },
    history: async (tab, action) => {
      await request(`${tabs}/${encodeURIComponent(tab)}/history`, { method: 'POST', ...jsonBody({ action } satisfies TabHistory) })
    },
    stream: liveStreamAt(stream.toString()),
  }
}
