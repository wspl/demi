import { reactive, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { reportError } from '@demicodes/web-ui/infra/errors'
import {
  afterDecision,
  type PermissionDecision,
  type PermissionGrantView,
  type PermissionRequestView,
} from '@demicodes/web-ui/permissions/types'
import { ApiError } from '../api/client'
import { decidePermission, readPermissions, revokePermission } from '../api/permissions'
import type { ConversationPermissions, PermissionCategory } from '../api/generated/web-api'
import { useProduct } from '../state/product'

/** One conversation's permission requests and grants, as this page last read them. */
export interface PermissionsState {
  /** The revision of the answer the page holds; -1 before the first. */
  revision: number
  requests: PermissionRequestView[]
  grants: PermissionGrantView[]
  load: 'loading' | 'ready' | 'failed'
  /** A decision of this page is on its way. */
  deciding: boolean
  /** The categories whose revocation is on its way. */
  revoking: string[]
}

function category(answer: PermissionCategory): PermissionRequestView['category'] {
  return {
    id: answer.id,
    action: answer.action ?? null,
    description: answer.description ?? null,
  }
}

/**
 * The conversations' permission requests and grants (`web-application.md`
 * § Synchronized state, `web-api.md` § Conversation permissions): a page
 * reads them for a conversation it shows, and again whenever the summary's
 * `permissionsRevision` rises past the revision it holds; an answer older
 * than the one it holds is dropped, since a read and the summary can arrive
 * in either order. A decision shows at once; another page's decision
 * reaches this one through the summary. Also the conversation whose
 * Permissions dialog is open.
 */
export const usePermissions = defineStore('permissions', () => {
  const product = useProduct()
  const states = reactive(new Map<string, PermissionsState>())
  /** The conversation whose Permissions dialog is open. */
  const dialog = ref<string | null>(null)

  function stateFor(conversationId: string): PermissionsState {
    if (!states.has(conversationId)) {
      states.set(conversationId, {
        revision: -1,
        requests: [],
        grants: [],
        load: 'loading',
        deciding: false,
        revoking: [],
      })
    }
    return states.get(conversationId)!
  }

  /** Takes an answer newer than the one the page holds. */
  function take(state: PermissionsState, answer: ConversationPermissions): void {
    if (answer.revision <= state.revision) {
      return
    }
    state.revision = answer.revision
    state.requests = answer.requests.map((request) => ({
      id: request.id,
      category: category(request.category),
      command: request.command,
      subagent: request.agent,
    }))
    state.grants = answer.grants.map((grant) => ({
      category: category(grant.category),
      grantedAt: grant.grantedAt,
    }))
  }

  async function read(conversationId: string): Promise<void> {
    const state = stateFor(conversationId)
    try {
      take(state, await readPermissions(conversationId))
      state.load = 'ready'
    } catch (error) {
      state.load = 'failed'
      reportError('Could Not Read Permissions', error)
    }
  }

  /** Reads the conversation's permissions once a page shows it; the summary keeps them current from then on. */
  function follow(conversationId: string): void {
    if (!states.has(conversationId)) {
      void read(conversationId)
    }
  }

  watch(
    () => product.snapshot?.conversations,
    (summaries) => {
      for (const summary of summaries ?? []) {
        const state = states.get(summary.id)
        if (state && summary.permissionsRevision > state.revision) {
          void read(summary.id)
        }
      }
    },
  )

  async function decide(
    conversationId: string,
    requestId: string,
    decision: PermissionDecision,
  ): Promise<void> {
    const state = stateFor(conversationId)
    state.deciding = true
    state.requests = afterDecision(state.requests, requestId, decision)
    try {
      await decidePermission(conversationId, requestId, decision)
    } catch (error) {
      // Another page decided it first: the summary brings its outcome.
      if (!(error instanceof ApiError && error.code === 'permission_request_not_found')) {
        reportError('Could Not Answer the Request', error, { userVisible: true })
      }
      await read(conversationId)
    } finally {
      state.deciding = false
    }
  }

  async function revoke(conversationId: string, categoryId: string): Promise<void> {
    const state = stateFor(conversationId)
    state.revoking = [...state.revoking, categoryId]
    try {
      await revokePermission(conversationId, categoryId)
      state.grants = state.grants.filter((grant) => grant.category.id !== categoryId)
    } catch (error) {
      reportError('Could Not Revoke the Permission', error, { userVisible: true })
    } finally {
      state.revoking = state.revoking.filter((id) => id !== categoryId)
    }
  }

  function openDialog(conversationId: string): void {
    dialog.value = conversationId
    void read(conversationId)
  }

  function closeDialog(): void {
    dialog.value = null
  }

  return { stateFor, follow, read, decide, revoke, dialog, openDialog, closeDialog }
})
