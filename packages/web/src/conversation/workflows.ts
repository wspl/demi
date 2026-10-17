import { computed, onScopeDispose, shallowRef, watch, type ComputedRef } from 'vue'
import type { Showing } from '@demicodes/web-ui/files/file-cache'
import type { WorkflowList } from '@demicodes/web-ui/agent/workflow-menu'
import { followOnScreen } from '@demicodes/web-ui/files/on-screen'
import { t } from './locales/text'
import { apiRequest, readResponse } from '../api/client'
import { fileBrowserError, hostFiles, readAtOf } from '../api/files'
import { conversationWorkflowsSchema, type ConversationWorkflows } from '../api/generated/web-api'
import { deviceWatch } from './file-watch'
import { conversationWatch } from './files'
import { executionFor } from '../targets/execution'
import { useProduct } from '../state/product'
import { useConversations } from './store'
import type { Conversation } from '../state/types'

/** Where a conversation's workflows are: the device and directory it works in. */
interface WorkflowPlace {
  deviceId: string
  directory: string
  /**
   * The conversation, once it has a record, whose host access reads them;
   * null for a new conversation before its first message, which reads them
   * through the device (`subagents.md` § Workflows of a project).
   */
  conversationId: string | null
}

/** A place with no workflows to read, as a new conversation on the Cloud before its directory exists. */
const NONE: WorkflowList = { state: 'nowhere' }

/**
 * The workflows of `place`, as the Workflow menu lists them: read from the
 * conversation's `GET /conversations/:id/workflows`, or a new
 * conversation's `GET /devices/:id/workflows`, and kept with the Host's
 * other files by the directory, subscribed on the file watch to what the
 * answer names, so a workflow added, changed or removed reads the list
 * again, with no polling. A conversation that moves to another directory of
 * the same Host reads that directory's, and lets go of the last. Shown while
 * the calling component lives, and followed while it is on screen. A new
 * conversation's remembered workflow that the list lacks settles to No
 * Subagents.
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
  const showing = shallowRef<Showing<ConversationWorkflows> | null>(null)
  const shown = computed(() => {
    const value = place()
    // One key per place, so the watch below sees a change only when it is one.
    return value ? JSON.stringify([value.deviceId, value.directory, value.conversationId]) : null
  })
  const stop = watch(shown, (key, _previous, onCleanup) => {
    showing.value = null
    const at = place()
    if (key === null || at === null)
      return
    const files = hostFiles(at.deviceId)
    const { conversationId, deviceId, directory } = at
    const held = files.show<ConversationWorkflows>({
      kind: 'answer',
      path: directory,
      key: 'workflows',
      watch: conversationId === null
        ? deviceWatch(deviceId, files)
        : conversationWatch(conversationId).channel(deviceId, files),
      // A read after the page's watch went live asks for an answer read at
      // or after it, which the backend's kept search then searches again
      // for (`web-api-conversations.md` § Workflows of a directory).
      async read(_held, after) {
        const query = new URLSearchParams(conversationId === null ? { directory } : {})
        if (after !== null)
          query.set('readAfter', `${after.epoch}.${after.number}`)
        const search = query.size > 0 ? `?${query}` : ''
        const route = conversationId === null
          ? `/devices/${encodeURIComponent(deviceId)}/workflows${search}`
          : `/conversations/${encodeURIComponent(conversationId)}/workflows${search}`
        try {
          const response = await apiRequest(route)
          const readAt = readAtOf(response.headers)
          return { value: await readResponse(response, conversationWorkflowsSchema), readAt }
        } catch (error) {
          throw fileBrowserError(error)
        }
      },
      size: (value) => JSON.stringify(value).length,
      subjects: (value) => value?.watch ?? [],
    })
    showing.value = held
    onCleanup(() => held.release())
  }, { immediate: true })
  onScopeDispose(stop)
  // The list follows the Host only while the composer is on screen.
  followOnScreen(() => [showing.value])
  const list = computed<WorkflowList>(() => {
    if (shown.value === null)
      return NONE
    const state = showing.value?.entry
    if (!state || (state.answer === null && state.failure === null))
      return { state: 'reading' }
    if (state.answer === 'value' && state.value) {
      const { workflows, unavailable } = state.value
      return unavailable === null ? { state: 'read', workflows } : { state: 'unavailable', reason: unavailable }
    }
    return { state: 'failed', reason: state.failure?.message ?? t('workflows.hostSilent') }
  })
  // A new conversation that started with a workflow its directory does not
  // have goes on with No Subagents once the workflows are read.
  const stopSettling = watch(
    () => [conversation().id, conversation().workflow, list.value] as const,
    ([id, , read]) => store.settleStartingWorkflow(id, read),
    { immediate: true },
  )
  onScopeDispose(stopSettling)
  return list
}
