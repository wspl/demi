import { z } from 'zod'
import type { ReadCallChange } from '@demicodes/web-ui/files/changes'
import type { ChangeReads, HistoryReads } from '@demicodes/web-ui/files/kept-source'
import { ApiError, apiRequest, readResponse } from '../api/client'
import { changeSidesSchema, gitBranchesSchema, gitCommitSchema, gitLogSchema, workingTreeChangesSchema } from '../api/generated/web-api'
import { fileBrowserError, rawFileContents, readAtOf } from '../api/files'

/**
 * How the page reads the uncommitted changes of a working tree, the
 * conversation's execution directory or the `directory` a child works in,
 * each a request: the list, both sides of one file, and the committed side's
 * bytes. The conversation's files service keeps what they answer.
 */
export function workingTreeReads(conversationId: string, directory?: string): ChangeReads {
  const id = encodeURIComponent(conversationId)
  // The routes read the conversation's directory unless a request names another.
  const tree: Record<string, string> = directory === undefined ? {} : { directory }
  return {
    async list() {
      try {
        const response = await apiRequest(`/conversations/${id}/changes${directory === undefined ? '' : `?${new URLSearchParams(tree)}`}`)
        const readAt = readAtOf(response.headers)
        const changes = await readResponse(response, workingTreeChangesSchema)
        const list = {
          files: changes.files,
          truncated: changes.truncated,
          repository: changes.repository,
        }
        return { value: list, readAt }
      } catch (error) {
        throw fileBrowserError(error)
      }
    },
    async sides(path, from) {
      try {
        const query = new URLSearchParams({ path, ...(from === undefined ? {} : { from }), ...tree })
        const response = await apiRequest(`/conversations/${id}/changes/file?${query}`)
        const readAt = readAtOf(response.headers)
        const sides = await readResponse(response, changeSidesSchema)
        return { value: sides, readAt }
      } catch (error) {
        // A side that is not text is an answer, which the views show another way.
        throw fileBrowserError(error)
      }
    },
    committed: rawFileContents(`/conversations/${id}/changes/raw`, tree),
  }
}

/**
 * How the page reads the history of the repository a working tree is in,
 * the conversation's execution directory or the `directory` a child works
 * in, each a request: its branches, a page of a branch's commits, one commit,
 * one of its files' two sides, and a commit's files' bytes.
 */
export function historyReads(conversationId: string, directory?: string): HistoryReads {
  const id = encodeURIComponent(conversationId)
  const tree: Record<string, string> = directory === undefined ? {} : { directory }
  async function read<T>(route: string, query: Record<string, string>, schema: z.ZodType<T>) {
    try {
      const search = new URLSearchParams({ ...query, ...tree })
      const response = await apiRequest(`/conversations/${id}/changes/${route}${search.size > 0 ? `?${search}` : ''}`)
      return { value: await readResponse(response, schema), readAt: readAtOf(response.headers) }
    } catch (error) {
      // A branch deleted or a commit the repository lacks is an answer, not found.
      throw fileBrowserError(error)
    }
  }
  return {
    branches: () => read('branches', {}, gitBranchesSchema),
    log: (branch, after) => read('log', { ...(branch === null ? {} : { branch }), ...(after === '' ? {} : { after }) }, gitLogSchema),
    commit: (commit) => read('commit', { commit }, gitCommitSchema),
    sides: (commit, path, from) => read('file', { commit, path, ...(from === undefined ? {} : { from }) }, changeSidesSchema),
    contents: (commit) => rawFileContents(`/conversations/${id}/changes/raw`, { ...tree, commit }),
  }
}

/**
 * The texts of the blobs read so far, for the page's lifetime: a blob's bytes
 * never change, so the Change view reads each once to draw its diffs. A
 * read that fails is dropped, so the next one tries again.
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
 * its view names (`crates/agent-store/src/media.rs`), without the Host; null
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
