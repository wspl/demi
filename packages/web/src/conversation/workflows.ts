import { computed, onScopeDispose, shallowRef, watch, type ComputedRef } from 'vue'
import type { WorkflowList } from '@demicodes/web-ui/agent/workflow-menu'
import { t } from './locales/text'
import { apiRequest, readResponse } from '../api/client'
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
   * keeps; null for a new conversation before its first message, which reads
   * them through the device (`crates/backend-user-shard/src/conversation/workflows.rs`).
   */
  conversationId: string | null
}

/** A place with no workflows to read, as a new conversation on the Cloud before its directory exists. */
const NONE: WorkflowList = { state: 'nowhere' }

/** The menu's list of what the backend answered. */
function listOf(kept: KeptWorkflowList): WorkflowList {
  if (kept.state === 'unknown') {
    return { state: 'reading' }
  }
  const { workflows, unavailable } = kept.list
  return unavailable === null ? { state: 'read', workflows } : { state: 'unavailable', reason: unavailable }
}

/**
 * The workflows of `place`, as the Workflow menu lists them: what the backend
 * keeps of the conversation's working directory, `GET /conversations/:id/workflows`,
 * read again when its metadata names a newer `workflowsRevision`, which the
 * backend raises when the Host reports a change; or a new conversation's,
 * `GET /devices/:id/workflows`, read once for its directory.
 *
 * - The page never follows the Host for them: the backend answers at once
 *   with what it read, or that it has read nothing yet.
 * - A conversation that moves to another directory reads that directory's.
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
  /** One key per place, so the watch below sees a change only when it is one. */
  const placeKey = computed(() => {
    const value = place()
    return value ? JSON.stringify([value.deviceId, value.directory, value.conversationId]) : null
  })
  const revision = computed(() =>
    place()?.conversationId == null ? 0 : conversation().metadata.shown.value?.workflowsRevision ?? 0)
  const list = shallowRef<WorkflowList>(NONE)
  const stop = watch([placeKey, revision], ([key], before, onCleanup) => {
    const at = place()
    if (key === null || at === null) {
      list.value = NONE
      return
    }
    // A new place has nothing to show until it is read; a new revision of
    // the same place shows the last list meanwhile.
    if (before?.[0] !== key) {
      list.value = { state: 'reading' }
    }
    const abort = new AbortController()
    onCleanup(() => abort.abort())
    const route = at.conversationId === null
      ? `/devices/${encodeURIComponent(at.deviceId)}/workflows?${new URLSearchParams({ directory: at.directory })}`
      : `/conversations/${encodeURIComponent(at.conversationId)}/workflows`
    apiRequest(route, { signal: abort.signal })
      .then((response) => readResponse(response, keptWorkflowListSchema))
      .then(
        (kept) => {
          list.value = listOf(kept)
        },
        (error: unknown) => {
          // A read the next place or revision replaced says nothing.
          if (!abort.signal.aborted) {
            const failure = fileBrowserError(error)
            list.value = { state: 'failed', reason: failure instanceof Error ? failure.message : t('workflows.hostSilent') }
          }
        },
      )
  }, { immediate: true })
  onScopeDispose(stop)
  // A new conversation that started with a workflow its directory does not
  // have goes on with No Subagents once the workflows are read.
  const stopSettling = watch(
    () => [conversation().id, conversation().workflow, list.value] as const,
    ([id, , read]) => store.settleStartingWorkflow(id, read),
    { immediate: true },
  )
  onScopeDispose(stopSettling)
  return computed(() => list.value)
}
