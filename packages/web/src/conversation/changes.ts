import { reactive } from 'vue'
import type { ShellFileChange } from '@demicodes/agent'
import type { ChangeSetSource } from '@demicodes/web-ui/files/changes'
import { ApiError, apiRequest, readResponse } from '../api/client'
import { changeSidesSchema, workingTreeChangesSchema } from '../api/contracts'

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
    files: [] as ShellFileChange[],
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
      const response = await apiRequest(
        `/conversations/${id}/changes/file?${new URLSearchParams({ path })}`,
        { signal },
      )
      return readResponse(response, changeSidesSchema)
    },
  })
  return source
}

function failureText(error: unknown): string {
  if (error instanceof ApiError) {
    switch (error.code) {
      case 'device_offline':
        return 'The device is offline.'
      case 'changes_busy':
        return 'The device is busy; try again in a moment.'
      case 'changes_timeout':
        return 'Listing the changes took too long.'
      default:
        return error.message
    }
  }
  return error instanceof Error ? error.message : String(error)
}
