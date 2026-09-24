import { reactive } from 'vue'
import type { ChangeSetSource, ReadCallChange, WorkingTreeChange } from '@demicodes/web-ui/files/changes'
import { ApiError, apiRequest, readResponse } from '../api/client'
import { changeSidesSchema, workingTreeChangesSchema } from '../api/generated/web-api'
import { fileBrowserError, rawFileContents } from '../api/files'

/**
 * The uncommitted changes of a conversation's execution directory, as the
 * change view's Uncommitted source: the list the backend last gave, and
 * both sides of any file on request (`web-api.md` § File text and working
 * tree changes). Listing again is explicit (`refresh`), coalesced while one
 * is in flight; a failed listing keeps the last list and says why. The host
 * marks the list stale when something may have changed while the view was
 * away, and `ensureFresh` lists again only then.
 */
export interface WorkingTreeSource extends ChangeSetSource {
  refresh(): void
  /** Lists again when the list was never taken or was marked stale. */
  ensureFresh(): void
  markStale(): void
}

export function createWorkingTreeSource(conversationId: string): WorkingTreeSource {
  const id = encodeURIComponent(conversationId)
  let stale = true
  let inFlight: Promise<void> | null = null
  let again = false

  async function list(): Promise<void> {
    source.refreshing = true
    try {
      const response = await apiRequest(`/conversations/${id}/changes`)
      const changes = await readResponse(response, workingTreeChangesSchema)
      source.files = changes.files
      source.truncated = changes.truncated
      source.unavailable = changes.repository ? null : 'no-repository'
      source.failure = null
      stale = false
    } catch (error) {
      source.failure = failureText(error)
    } finally {
      source.refreshing = false
    }
  }

  function refresh(): void {
    if (inFlight) {
      again = true
      return
    }
    inFlight = list().finally(() => {
      inFlight = null
      if (again) {
        again = false
        refresh()
      }
    })
  }

  const source: WorkingTreeSource = reactive({
    files: [] as WorkingTreeChange[],
    truncated: false,
    unavailable: null,
    refreshing: false,
    failure: null,
    refresh,
    ensureFresh() {
      if (stale) {
        refresh()
      }
    },
    markStale() {
      stale = true
    },
    async read(path: string, signal?: AbortSignal) {
      try {
        const response = await apiRequest(
          `/conversations/${id}/changes/file?${new URLSearchParams({ path })}`,
          { signal },
        )
        return await readResponse(response, changeSidesSchema)
      } catch (error) {
        // A side that is not text is the views' to show another way.
        throw fileBrowserError(error)
      }
    },
    committed: rawFileContents(`/conversations/${id}/changes/raw`),
  })
  return source
}

function failureText(error: unknown): string {
  if (error instanceof ApiError) {
    switch (error.code) {
      case 'device_offline':
        return 'The device is offline.'
      case 'changes_timeout':
        return 'Listing the changes took too long.'
      default:
        return error.message
    }
  }
  return error instanceof Error ? error.message : String(error)
}

/** Reads published conversation history without contacting its execution host. */
export function createCallChangeReader(conversationId: string): ReadCallChange {
  return async (commandId, path, edit, signal) => {
    try {
      const query = new URLSearchParams({ path, edit: String(edit) })
      const response = await apiRequest(
        `/conversations/${encodeURIComponent(conversationId)}/commands/${encodeURIComponent(commandId)}/changes/file?${query}`,
        { signal },
      )
      return readResponse(response, changeSidesSchema)
    } catch (error) {
      if (error instanceof ApiError && error.code === 'not_found') {
        return null
      }
      throw error
    }
  }
}
