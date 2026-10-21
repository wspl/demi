import { computed, onScopeDispose, shallowRef, watch, type ComputedRef } from 'vue'
import type { WorkflowList } from '@demicodes/web-ui/agent/workflow-menu'
import { t } from './locales/text'
import { ApiError, apiRequest, readResponse } from '../api/client'
import { fileBrowserError } from '../api/files'
import { keptWorkflowListSchema, type KeptWorkflowList } from '../api/generated/web-api'
import { executionFor } from '../targets/execution'
import { useProduct } from '../state/product'
import { useConversations } from './store'
import type { Conversation } from '../state/types'

/** Where a conversation's workflows are: the device and directory it works in. */
interface WorkflowPlace {
  deviceId: string
  directory: string
  /**
   * The conversation, once it has a record, whose workflows the backend
   * keeps and its metadata carries; null for a new conversation before its
   * first message, which reads them through the device
   * (`crates/backend-user-shard/src/conversation/workflows.rs`).
   */
  conversationId: string | null
}

/** A place with no workflows to read, as a new conversation on the Cloud before its directory exists. */
const NONE: WorkflowList = { state: 'nowhere' }

/** The menu's list of what the backend keeps. */
function listOf(kept: KeptWorkflowList): WorkflowList {
  switch (kept.state) {
    case 'unknown':
      return { state: 'reading' }
    case 'refused':
      return failed(new ApiError(kept.status, kept.code, kept.message))
    case 'read': {
      const { workflows, unavailable } = kept.list
      return unavailable === null ? { state: 'read', workflows } : { state: 'unavailable', reason: unavailable }
    }
  }
}

/** The menu's list of a read that failed with `error`, in the file browser's words. */
function failed(error: unknown): WorkflowList {
  const failure = fileBrowserError(error)
  return { state: 'failed', reason: failure instanceof Error ? failure.message : t('workflows.hostSilent') }
}

/**
 * The workflows of `place`, as the Workflow menu lists them: what the backend
 * keeps of the conversation's working directory, which its metadata carries
 * and brings again when the Host reports a change
 * (`docs/decisions/0029-a-conversation-opens-whole.md`); or a new
 * conversation's, `GET /devices/:id/workflows`, read once for its directory.
 *
 * - The page never follows the Host for them: the backend holds what it
 *   read, or that it has read nothing yet.
 * - A conversation that moves to another directory shows that directory's.
 * - A new conversation's remembered workflow that the list lacks settles to No
 *   Subagents.
 */
export function useConversationWorkflows(conversation: () => Conversation): ComputedRef<WorkflowList> {
  const product = useProduct()
  const store = useConversations()
  const place = (): WorkflowPlace | null => {
    const shown = conversation()
    const { deviceId, path, directory } = executionFor(shown)
    if (deviceId === null) {
      return null
    }
    const recorded = product.snapshot?.conversations.some((item) => item.id === shown.id) ?? false
    if (recorded) {
      return { deviceId, directory: path, conversationId: shown.id }
    }
    // Before its first message, a conversation reads them through its
    // device, the Cloud included, in the directory it chose or its
    // project's; one outside projects on the Cloud has none yet.
    return directory !== null ? { deviceId, directory, conversationId: null } : null
  }
  /** One key per new conversation's place, so the watch below sees a change only when it is one. */
  const unrecordedKey = computed(() => {
    const value = place()
    return value && value.conversationId === null ? JSON.stringify([value.deviceId, value.directory]) : null
  })
  const read = shallowRef<WorkflowList>(NONE)
  const stop = watch(unrecordedKey, (key, _before, onCleanup) => {
    const at = place()
    if (key === null || at === null) {
      read.value = NONE
      return
    }
    read.value = { state: 'reading' }
    const abort = new AbortController()
    onCleanup(() => abort.abort())
    const route = `/devices/${encodeURIComponent(at.deviceId)}/workflows?${new URLSearchParams({ directory: at.directory })}`
    apiRequest(route, { signal: abort.signal })
      .then((response) => readResponse(response, keptWorkflowListSchema))
      .then(
        (kept) => {
          read.value = listOf(kept)
        },
        (error: unknown) => {
          // A read the next place replaced says nothing.
          if (!abort.signal.aborted) {
            read.value = failed(error)
          }
        },
      )
  }, { immediate: true })
  onScopeDispose(stop)
  const list = computed<WorkflowList>(() => {
    const at = place()
    if (at === null) {
      return NONE
    }
    if (at.conversationId === null) {
      return read.value
    }
    const kept = conversation().metadata.shown.value?.workflows.list
    return kept ? listOf(kept) : { state: 'reading' }
  })
  // A new conversation that started with a workflow its directory does not
  // have goes on with No Subagents once the workflows are read.
  const stopSettling = watch(
    () => [conversation().id, conversation().workflow, list.value] as const,
    ([id, , shown]) => store.settleStartingWorkflow(id, shown),
    { immediate: true },
  )
  onScopeDispose(stopSettling)
  return list
}
