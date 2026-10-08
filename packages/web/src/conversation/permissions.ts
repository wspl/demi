import { reactive, watch } from 'vue'
import { defineStore } from 'pinia'
import { reportError } from '@demicodes/web-ui/infra/errors'
import {
  afterDecision,
  type PermissionDecision,
  type PermissionRequestView,
} from '@demicodes/web-ui/permissions/types'
import { ApiError } from '../api/client'
import { decidePermission, readPermissions } from '../api/permissions'
import type { ConversationPermissions, PermissionCategory } from '../api/generated/web-api'
import { useProduct } from '../state/product'
import { isNewer, type RunRevision } from '../state/revisions'

/** One conversation's permission requests, as this page last read them. */
export interface PermissionsState {
  /** The revision of the answer the page holds; null before the first. */
  held: RunRevision | null
  requests: PermissionRequestView[]
  /** A decision of this page is on its way. */
  deciding: boolean
}

function category(answer: PermissionCategory): PermissionRequestView['categories'][number] {
  return {
    id: answer.id,
    action: answer.action ?? null,
    description: answer.description ?? null,
  }
}

/**
 * The conversations' permission requests (`web-application.md`
 * § Synchronized state, `web-api.md` § Conversation permissions): a page
 * reads them for a conversation it shows, and again whenever the summary's
 * `permissionsRevision` is newer than the revision it holds, a count of
 * another run of the backend included (`web-api.md` § Revisions counted in
 * memory); an answer older than the one it holds is dropped, since a read
 * and the summary can arrive in either order. A decision shows at once and
 * is not read back; one refused goes back to the requests as they were, and
 * another page's decision reaches this one through the summary.
 */
export const usePermissions = defineStore('permissions', () => {
  const product = useProduct()
  const states = reactive(new Map<string, PermissionsState>())

  function stateFor(conversationId: string): PermissionsState {
    if (!states.has(conversationId)) {
      states.set(conversationId, {
        held: null,
        requests: [],
        deciding: false,
      })
    }
    return states.get(conversationId)!
  }

  /** Takes an answer newer than the one the page holds; it is of the run the page holds now. */
  function take(state: PermissionsState, answer: ConversationPermissions): void {
    const taken = { run: product.snapshot?.run ?? null, revision: answer.revision }
    if (!isNewer(taken, state.held)) {
      return
    }
    state.held = taken
    state.requests = answer.requests.map((request) => ({
      id: request.id,
      categories: request.categories.map(category),
      command: request.command,
      subagent: request.agent,
    }))
  }

  /** The reads on their way, each with whether a newer revision was heard of meanwhile. */
  const reading = new Map<string, { again: boolean }>()

  /** Whether the summary names a revision newer than the one the page holds. */
  function behind(conversationId: string): boolean {
    const summary = product.snapshot?.conversations.find((item) => item.id === conversationId)
    const run = product.snapshot?.run ?? null
    return summary !== undefined && isNewer({ run, revision: summary.permissionsRevision }, stateFor(conversationId).held)
  }

  /**
   * Reads the requests, unless a read is on its way: that one is read again
   * after, if the summary is still newer than what it brought, so a page
   * that opens a conversation reads its requests once.
   */
  async function read(conversationId: string): Promise<void> {
    const running = reading.get(conversationId)
    if (running) {
      running.again = true
      return
    }
    const state = stateFor(conversationId)
    const current = { again: false }
    reading.set(conversationId, current)
    try {
      take(state, await readPermissions(conversationId))
    } catch (error) {
      reportError('Could Not Read Permissions', error)
    } finally {
      reading.delete(conversationId)
    }
    if (current.again && behind(conversationId)) {
      await read(conversationId)
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
        if (states.has(summary.id) && behind(summary.id)) {
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
    const before = { held: state.held, requests: state.requests }
    state.requests = afterDecision(state.requests, requestId, decision)
    try {
      await decidePermission(conversationId, requestId, decision)
    } catch (error) {
      // Another page decided it first: the summary brings its outcome.
      if (error instanceof ApiError && error.code === 'permission_request_not_found') {
        return
      }
      reportError('Could Not Answer the Request', error, { userVisible: true })
      // The request is as it was, unless a newer read came meanwhile.
      if (state.held === before.held) {
        state.requests = before.requests
      }
    } finally {
      state.deciding = false
    }
  }

  return { stateFor, follow, decide }
})
