import type { SearchRow } from '@demicodes/web-ui/search/search'
import { apiRequest, readResponse } from './client'
import { searchResultsSchema } from './generated/web-api'

/** The caller's conversations that match `query` (`web-api.md` § Search). */
export async function searchConversations(query: string, signal: AbortSignal): Promise<SearchRow[]> {
  const response = await apiRequest(`/search?${new URLSearchParams({ q: query })}`, { signal })
  return (await readResponse(response, searchResultsSchema)).results
}
