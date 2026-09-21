import {
  BrowserTabsError,
  browserTabInfoSchema,
  browserTabListSchema,
  type BrowserTabsApi,
} from '@demicodes/web-ui/browser/tabs'
import { liveStreamAt } from '@demicodes/web-ui/transport/live-stream'
import { reportActivity } from './activity'
import { ApiError, apiRequest, apiUrl, jsonBody, readResponse } from './client'

/** A refusal keeps the backend's own code and message for the tab's content to show. */
async function request(path: string, init?: RequestInit): Promise<Response> {
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
    list: async () => readResponse(await request(tabs), browserTabListSchema),
    open: async (url) => readResponse(
      await request(tabs, { method: 'POST', ...jsonBody({ url }) }),
      browserTabInfoSchema,
    ),
    close: async (tab) => {
      await request(`${tabs}/${encodeURIComponent(tab)}`, { method: 'DELETE' })
    },
    navigate: async (tab, url) => {
      await request(`${tabs}/${encodeURIComponent(tab)}/navigate`, { method: 'POST', ...jsonBody({ url }) })
    },
    history: async (tab, action) => {
      await request(`${tabs}/${encodeURIComponent(tab)}/history`, { method: 'POST', ...jsonBody({ action }) })
    },
    stream: liveStreamAt(stream.toString()),
    onOperation: () => void reportActivity(conversationId),
  }
}
