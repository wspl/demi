import type { FoundMessages, SearchRow } from '@demicodes/web-ui/search/search'
import { apiRequest, readResponse } from './client'
import { messageResultsSchema, searchResultsSchema } from './generated/web-api'

/** The caller's conversations that match `query`: of the workspace `workspace`, or of every one without it. */
export async function searchConversations(query: string, signal: AbortSignal, workspace?: string | null): Promise<SearchRow[]> {
  const params = new URLSearchParams({ q: query, ...(workspace ? { workspace } : {}) })
  const response = await apiRequest(`/search?${params}`, { signal })
  return (await readResponse(response, searchResultsSchema)).results
}

/** The messages of the conversation `id` that match `query`, newest first. */
export async function searchConversation(id: string, query: string, signal: AbortSignal): Promise<FoundMessages> {
  const path = `/conversations/${encodeURIComponent(id)}/search?${new URLSearchParams({ q: query })}`
  const { messages, limited } = await readResponse(await apiRequest(path, { signal }), messageResultsSchema)
  return { rows: messages, limited }
}
