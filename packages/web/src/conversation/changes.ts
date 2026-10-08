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
  // Each read names the version the page holds: an unchanged answer is a
  // 304 without its content.
  const held = (version: string | null) => version === null
    ? { allowNotModified: true }
    : { headers: { 'if-none-match': version }, allowNotModified: true }
  return {
    async list(version) {
      try {
        const response = await apiRequest(`/conversations/${id}/changes`, held(version))
        if (response.status === 304)
          return null
        const changes = await readResponse(response, workingTreeChangesSchema)
        return {
          files: changes.files,
          truncated: changes.truncated,
          repository: changes.repository,
          gitDir: changes.gitDir,
          version: response.headers.get('etag'),
        }
      } catch (error) {
        throw fileBrowserError(error)
      }
    },
    async sides(path, version) {
      try {
        const response = await apiRequest(`/conversations/${id}/changes/file?${new URLSearchParams({ path })}`, held(version))
        if (response.status === 304)
          return null
        return { ...await readResponse(response, changeSidesSchema), version: response.headers.get('etag') }
      } catch (error) {
        // A side that is not text is the views' to show another way.
        throw fileBrowserError(error)
      }
    },
    committed: rawFileContents(`/conversations/${id}/changes/raw`),
  }
}

/**
 * The texts of the blobs read so far, for the page's lifetime: a blob's bytes
 * never change, so the Change view and a work group's file counts read each
 * once. A read that fails is dropped, so the next one tries again.
 */
const blobTexts = new Map<string, Promise<string>>()

function blobText(blob: string): Promise<string> {
  let text = blobTexts.get(blob)
  if (!text) {
    text = apiRequest(`/blobs/${encodeURIComponent(blob)}`).then((response) => response.text())
    blobTexts.set(blob, text)
    text.catch(() => blobTexts.delete(blob))
  }
  return text
}

/**
 * Reads both sides of a tool call's edit from the blob route, by the blobs
 * its view names (`edit-tracking.md` § Edit copies), without the Host; null
 * when the user's namespace no longer holds one of them. The copies are
 * text, since only text is stored. The read itself is shared and kept, so
 * `signal` only stops this caller's wait.
 */
export const readEditCopies: ReadCallChange = async (copies, signal) => {
  try {
    const [original, modified] = await Promise.all([blobText(copies.original), blobText(copies.modified)])
    signal?.throwIfAborted()
    return { original, modified }
  } catch (error) {
    if (error instanceof ApiError && error.code === 'not_found') {
      return null
    }
    throw error
  }
}
