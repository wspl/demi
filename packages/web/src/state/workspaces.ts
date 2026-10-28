import { computed, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue } from '@demicodes/utils'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { newWorkspaceName, type WorkspaceView } from '@demicodes/web-ui/workspaces/workspaces'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  workspaceAnswerSchema,
  type ConversationTarget,
  type ChangeWorkspace,
  type CreateWorkspace,
  type WorkspaceMove,
  type WorkspaceOrder,
} from '../api/generated/web-api'
import { useConversations } from '../conversation/store'
import { t } from './locales/text'
import { known, useProduct } from './product'
import { useResources } from './resources'

/**
 * The account's workspaces and the one this tab shows.
 *
 * A tab's workspace is the workspace of the conversation it shows, a new one
 * included, which keeps the workspace it was started in. A tab that shows no
 * conversation shows the workspace this browser last switched to, or else the
 * account's first. So two tabs show two workspaces, and the address names
 * none: a conversation's id says its workspace.
 */
export const useWorkspaces = defineStore('workspaces', () => {
  const product = useProduct()
  const resources = useResources()
  /** The conversation the tab shows; null for none (`App.vue`). */
  const shownConversation = ref<string | null>(null)
  /** The workspace Settings › Workspaces shows; null for the tab's. */
  const selected = ref<string | null>(null)
  /** A workspace just made, whose name Settings › Workspaces shows selected, ready to type over. */
  const naming = ref<string | null>(null)

  const list = computed((): WorkspaceView[] =>
    (product.snapshot?.workspaces ?? []).map((workspace) => ({
      id: workspace.id,
      name: workspace.name ?? null,
      icon: workspace.icon,
      color: workspace.color,
    })),
  )

  const current = computed((): string | null => {
    const ids = new Set(list.value.map((workspace) => workspace.id))
    const conversations = useConversations()
    const shown = conversations.items.find((item) => item.id === shownConversation.value)
    if (shown && ids.has(shown.workspaceId)) {
      return shown.workspaceId
    }
    const last = resources.local.lastWorkspace
    if (last && ids.has(last)) {
      return last
    }
    return list.value[0]?.id ?? null
  })

  /** The workspaces with a conversation that waits for the user, as its sidebar row's dot says. */
  const attention = computed((): string[] => {
    const conversations = useConversations()
    const waiting = new Set<string>()
    for (const item of conversations.items) {
      if (!item.archived && (item.needsYou || item.unread)) {
        waiting.add(item.workspaceId)
      }
    }
    return list.value.filter((workspace) => waiting.has(workspace.id)).map((workspace) => workspace.id)
  })

  /** Remembers `conversation` as the one this browser last showed in its workspace. */
  function showed(conversation: { id: string; workspaceId: string }): void {
    const kept = resources.local.workspaceConversations ?? {}
    if (kept[conversation.workspaceId] !== conversation.id) {
      resources.local.workspaceConversations = { ...kept, [conversation.workspaceId]: conversation.id }
    }
  }

  /**
   * The conversation a switch to `id` shows: the one this browser last showed
   * there, while it is still there and not archived, or else the
   * workspace's first in the sidebar; null for none, where a switch opens a
   * new conversation.
   */
  function landing(id: string): string | null {
    const conversations = useConversations()
    const inside = conversations.listed.filter((item) => item.workspaceId === id && !item.archived)
    const last = resources.local.workspaceConversations?.[id]
    return (last && inside.find((item) => item.id === last)?.id) ?? inside[0]?.id ?? null
  }

  /** Records `id` as the workspace this browser switched to last. */
  function chose(id: string): void {
    resources.local.lastWorkspace = id
  }

  const writes = new SerialQueue()
  /** The page's lifetime, which a wait for the channel ends with. */
  const lifetime = new AbortController()

  /** Makes a workspace with the next default name, and answers its id once the channel brings it. */
  async function create(): Promise<string> {
    const response = await apiRequest('/workspaces', {
      method: 'POST',
      ...jsonBody({ name: newWorkspaceName(list.value) } satisfies CreateWorkspace),
    })
    const { workspace } = await readResponse(response, workspaceAnswerSchema)
    await product.until((state) => state.workspaces?.some((item) => item.id === workspace.id) ?? false, lifetime.signal)
    return workspace.id
  }

  /** Applies `change` to the workspace `id` at once; the write follows, and a failure puts it back. */
  function change(id: string, edit: ChangeWorkspace): void {
    const changed = product.change('workspaces', (state) => ({
      ...state,
      workspaces: known(state.workspaces, 'workspaces').map((workspace) =>
        workspace.id === id ? { ...workspace, ...edit } : workspace,
      ),
    }))
    void writes
      .run(id, async () => {
        changed.send()
        const response = await apiRequest(`/workspaces/${encodeURIComponent(id)}`, {
          method: 'PATCH',
          ...jsonBody(edit satisfies ChangeWorkspace),
        })
        const { workspace } = await readResponse(response, workspaceAnswerSchema)
        changed.land((read) => ({
          type: 'workspaces',
          workspaces: known(read.workspaces, 'workspaces').map((candidate) =>
            candidate.id === workspace.id ? workspace : candidate,
          ),
        }))
      })
      .catch((error) => {
        changed.drop()
        reportError(t('failures.updateSettings'), error, { site: 'failures.updateSettings', userVisible: true })
      })
  }

  /** Puts the workspaces in the order of `ids` at once; the write follows, and a failure puts it back. */
  function reorder(ids: string[]): void {
    const changed = product.change('workspaces', (state) => {
      const workspaces = known(state.workspaces, 'workspaces')
      return {
        ...state,
        workspaces: ids.flatMap((id) => workspaces.filter((workspace) => workspace.id === id)),
      }
    })
    void writes
      .run('order', async () => {
        changed.send()
        await apiRequest('/workspaces/order', { method: 'PUT', ...jsonBody({ ids } satisfies WorkspaceOrder) })
        changed.land()
      })
      .catch((error) => {
        changed.drop()
        reportError(t('failures.updateSettings'), error, { site: 'failures.updateSettings', userVisible: true })
      })
  }

  /**
   * Deletes the workspace `id` with its conversations and projects, done once
   * the channel no longer lists it; the new conversations this page holds
   * there, which the backend never had, go with it (`settleDrafts`).
   */
  async function remove(id: string): Promise<void> {
    await apiRequest(`/workspaces/${encodeURIComponent(id)}`, { method: 'DELETE' })
    await product.until((state) => !(state.workspaces?.some((item) => item.id === id) ?? true), lifetime.signal)
  }

  /**
   * Moves conversations and projects into the workspace `id`. A new
   * conversation the backend does not have yet moves here, on the page: it is
   * in the workspace now, and one started in a project starts in its
   * directory, which on its first send selects the workspace's project of it.
   */
  async function move(id: string, moved: string[], projects: string[]): Promise<void> {
    const conversations = useConversations()
    const local = conversations.items.filter(
      (item) => item.persistence !== 'synced' && (moved.includes(item.id) || (item.projectId !== null && projects.includes(item.projectId))),
    )
    for (const draft of local) {
      draft.target = directoryOf(draft.projectId) ?? draft.target
      draft.projectId = null
      draft.workspaceId = id
    }
    const synced = moved.filter((conversation) => !local.some((draft) => draft.id === conversation))
    if (synced.length || projects.length) {
      await apiRequest(`/workspaces/${encodeURIComponent(id)}/moves`, {
        method: 'POST',
        ...jsonBody({ conversations: synced, projects } satisfies WorkspaceMove),
      })
    }
  }

  /** The directory of the project `id` as a target that names no project; none for none. */
  function directoryOf(id: string | null): ConversationTarget | null {
    const project = resources.projects.find((candidate) => candidate.id === id)
    if (!project) {
      return null
    }
    return project.hostKind === 'cloud'
      ? { kind: 'cloud', path: project.path }
      : { kind: 'device', deviceId: project.deviceId, path: project.path }
  }

  /**
   * The new conversations this page holds, once the workspaces are known: one
   * kept before workspaces, with none, takes the tab's; one whose workspace
   * was deleted goes, as the workspace's conversations went.
   */
  function settleDrafts(): void {
    const ids = new Set(list.value.map((workspace) => workspace.id))
    if (!ids.size) {
      return
    }
    const conversations = useConversations()
    const gone: string[] = []
    for (const item of conversations.items) {
      if (item.persistence === 'synced') {
        continue
      }
      if (item.workspaceId === '') {
        item.workspaceId = current.value ?? ''
      } else if (!ids.has(item.workspaceId)) {
        gone.push(item.id)
      }
    }
    if (gone.length) {
      void conversations.remove(gone)
    }
  }
  watch(
    () => [list.value.map((workspace) => workspace.id).join(), useConversations().items.length] as const,
    settleDrafts,
  )

  return { shownConversation, selected, naming, list, current, attention, showed, landing, chose, create, change, reorder, remove, move }
})
