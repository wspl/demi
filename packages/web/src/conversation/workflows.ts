import { computed, onScopeDispose, shallowRef, watch, type ComputedRef } from 'vue'
import type { Showing } from '@demicodes/web-ui/files/file-cache'
import type { WorkflowList } from '@demicodes/web-ui/agent/workflow-menu'
import { useOnScreen } from '@demicodes/web-ui/files/on-screen'
import { apiRequest, readResponse } from '../api/client'
import { fileBrowserError, hostFiles, readAtOf } from '../api/files'
import { conversationWorkflowsSchema, type ConversationWorkflows } from '../api/generated/web-api'
import { conversationWatch } from './files'

/**
 * The project's workflows of a conversation on its primary Host's device,
 * as the Workflow menu lists them (`subagents.md` § Workflows of a
 * project): read from `GET /conversations/:id/workflows` and kept with the
 * Host's other files, subscribed on the conversation's file watch to what
 * the answer names, so a workflow added, changed or removed reads the list
 * again, with no polling. Shown while the calling component lives, and
 * followed while it is on screen.
 */
export function useConversationWorkflows(
  conversationId: () => string,
  deviceId: () => string | null,
): ComputedRef<WorkflowList> {
  const onScreen = useOnScreen()
  const showing = shallowRef<Showing<ConversationWorkflows> | null>(null)
  const place = computed(() => {
    const device = deviceId()
    return device === null ? null : { id: conversationId(), device }
  })
  const stop = watch(place, (shown, _previous, onCleanup) => {
    showing.value = null
    if (shown === null)
      return
    const files = hostFiles(shown.device)
    const held = files.show<ConversationWorkflows>({
      kind: 'answer',
      path: `workflows:${shown.id}`,
      watch: conversationWatch(shown.id).channel(shown.device, files),
      async read() {
        try {
          const response = await apiRequest(`/conversations/${encodeURIComponent(shown.id)}/workflows`)
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
    const following = watch(onScreen, (on) => (on ? held.back() : held.away()), { immediate: true })
    onCleanup(() => {
      following()
      held.release()
    })
  }, { immediate: true })
  onScopeDispose(stop)
  return computed<WorkflowList>(() => {
    const state = showing.value?.entry
    if (!state || (state.answer === null && state.failure === null))
      return { state: 'reading' }
    if (state.answer === 'value' && state.value)
      return { state: 'read', workflows: state.value.workflows }
    return { state: 'failed', reason: state.failure?.message ?? 'The Host did not answer.' }
  })
}
