import { reactive, watch } from 'vue'
import { defineStore } from 'pinia'
import { reportError } from '@demicodes/web-ui/infra/errors'
import {
  afterDecision,
  type CredentialAnswer,
  type PermissionDecision,
  type PermissionRequestView,
  type RequestView,
} from '@demicodes/web-ui/permissions/types'
import { ApiError } from '../api/client'
import { decidePermission, readPermissions } from '../api/permissions'
import type { ConversationPermissions, PermissionCategory, PermissionRequest } from '../api/generated/web-api'
import { useProduct } from '../state/product'
import { isNewer, type RunRevision } from '../state/revisions'

/** One conversation's permission requests, as this page last read them. */
export interface PermissionsState {
  /** The revision of the answer the page holds; null before the first. */
  held: RunRevision | null
  requests: RequestView[]
  /** A decision of this page is on its way. */
  deciding: boolean
  /** What the backend refused the last credential answer for, shown in the card; null while none was. */
  error: string | null
}

function category(answer: PermissionCategory): PermissionRequestView['categories'][number] {
  return {
    id: answer.id,
    action: answer.action ?? null,
    description: answer.description ?? null,
  }
}

/** The request as the card shows it; a credential request is a renewal when a credential of its name is saved. */
function view(request: PermissionRequest, saved: ReadonlySet<string>): RequestView {
  switch (request.kind) {
    case 'permission':
      return {
        kind: 'permission',
        id: request.id,
        categories: request.categories.map(category),
        command: request.command,
        subagent: request.agent,
      }
    case 'credential':
      return {
        kind: 'credential',
        id: request.id,
        name: request.name,
        reason: request.reason,
        fields: request.fields,
        keep: request.keep,
        renewal: saved.has(credentialKey(request.name)),
      }
    case 'allowance':
      return { kind: 'allowance', id: request.id, name: request.name, removed: request.removed }
  }
}

/** A credential's name as the backend compares names: whatever the case (`web-api.md` § Credentials). */
function credentialKey(name: string): string {
  return name.trim().toLowerCase()
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
        error: null,
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
    const saved = new Set((product.snapshot?.credentials ?? []).map((credential) => credentialKey(credential.name)))
    state.requests = answer.requests.map((request) => view(request, saved))
    state.error = null
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

  /**
   * Decides a request. The card shows the outcome at once, except for a
   * credential's values: that card stays, form and typed values with it,
   * until the backend accepted them or told what is wrong with them.
   */
  async function decide(
    conversationId: string,
    requestId: string,
    decision: PermissionDecision,
    credential?: CredentialAnswer,
  ): Promise<void> {
    const state = stateFor(conversationId)
    state.deciding = true
    state.error = null
    const before = { held: state.held, requests: state.requests }
    if (!credential) {
      state.requests = afterDecision(state.requests, requestId, decision)
    }
    try {
      await decidePermission(conversationId, requestId, decision, credential)
      if (credential) {
        state.requests = afterDecision(state.requests, requestId, decision)
      }
    } catch (error) {
      if (error instanceof ApiError) {
        // Another page decided it first, or the credential was removed since: the summary brings the outcome.
        if (error.code === 'permission_request_not_found' || error.code === 'credential_removed') {
          return
        }
        // The values do not suit the request: said under the form, which keeps what was typed.
        if (error.code === 'invalid_answer' || error.code === 'credential_fields_differ') {
          state.error = error.message
          return
        }
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
