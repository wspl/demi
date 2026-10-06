import type { ReadCallChange } from '@demicodes/web-ui/files/changes'
import type { ChangeReads } from '@demicodes/web-ui/files/kept-source'
import { ApiError, apiRequest, readResponse } from '../api/client'
import { changeSidesSchema, workingTreeChangesSchema } from '../api/generated/web-api'
import { fileBrowserError, rawFileContents } from '../api/files'

/**
 * How the page reads the uncommitted changes of a conversation's execution
 * directory, each a request (`web-api.md` § File text and working tree
 * changes): the list, both sides of one file, and the committed side's
 * bytes. The conversation's files service keeps what they answer.
 */
export function workingTreeReads(conversationId: string): ChangeReads {
  const id = encodeURIComponent(conversationId)
  return {
    async list() {
      try {
        const response = await apiRequest(`/conversations/${id}/changes`)
        const changes = await readResponse(response, workingTreeChangesSchema)
        return { files: changes.files, truncated: changes.truncated, repository: changes.repository }
      } catch (error) {
        throw fileBrowserError(error)
      }
    },
    async sides(path) {
      try {
        const response = await apiRequest(`/conversations/${id}/changes/file?${new URLSearchParams({ path })}`)
        return await readResponse(response, changeSidesSchema)
      } catch (error) {
        // A side that is not text is the views' to show another way.
        throw fileBrowserError(error)
      }
    },
    committed: rawFileContents(`/conversations/${id}/changes/raw`),
  }
}

/**
 * Reads both sides of a tool call's edit from the blob route, by the blobs
 * its view names (`edit-tracking.md` § Edit copies), without the Host; null
 * when the user's namespace no longer holds one of them. The copies are
 * text, since only text is stored.
 */
export const readEditCopies: ReadCallChange = async (copies, signal) => {
  try {
    const [original, modified] = await Promise.all(
      [copies.original, copies.modified].map(async (blob) => {
        const response = await apiRequest(`/blobs/${encodeURIComponent(blob)}`, { signal })
        return response.text()
      }),
    )
    return { original, modified }
  } catch (error) {
    if (error instanceof ApiError && error.code === 'not_found') {
      return null
    }
    throw error
  }
}
