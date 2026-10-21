import { reactive, watch } from 'vue'
import { defineStore } from 'pinia'
import { codeText, formError, reportError, type ShownError } from '@demicodes/web-ui/infra/errors'
import { t } from './locales/text'
import {
  afterDecision,
  type CredentialAnswer,
  type PermissionDecision,
  type PermissionRequestView,
  type RequestView,
} from '@demicodes/web-ui/permissions/types'
import { ApiError } from '../api/client'
import { decidePermission } from '../api/permissions'
import type { ConversationPermissions, PermissionCategory, PermissionRequest } from '../api/generated/web-api'
import { useProduct } from '../state/product'
import { useConversations } from './store'
import { isNewer, type RunRevision } from '../state/revisions'

/** One conversation's permission requests, as its metadata last brought them. */
export interface PermissionsState {
  /** The revision of the answer the page holds; null before the first. */
  held: RunRevision | null
  requests: RequestView[]
  /** A decision of this page is on its way. */
  deciding: boolean
  /** What the backend refused the last credential answer for, shown in the card; null while none was. */
  error: ShownError | null
}

function category(answer: PermissionCategory): PermissionRequestView['categories'][number] {
  return {
    id: answer.id,
    action: answer.action ?? null,
    description: answer.description ?? null,
  }
}

/** The request as the card shows it. */
function view(request: PermissionRequest): RequestView {
  switch (request.kind) {
    case 'permission':
      return {
        kind: 'permission',
        id: request.id,
        categories: request.categories.map(category),
        host: request.host,
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
        renewal: request.renewal,
      }
    case 'allowance':
      return { kind: 'allowance', id: request.id, name: request.name, removed: request.removed }
  }
}

/**
 * Why the values in a credential card do not fit its request, in the page's
 * words: the field without a value, or the one over the size a value may
 * have; null for an answer the card did not send as it should, which the
 * catalogue's text for the refusal says.
 */
function answerProblem(error: ApiError): string | null {
  const { field, limitBytes } = error.params
  if (typeof field !== 'string') {
    return null
  }
  return typeof limitBytes === 'number'
    ? codeText('invalid_answer_too_large', { field, limitBytes })
    : codeText('invalid_answer_missing', { field })
}

/**
 * The conversations' permission requests, as the metadata of each conversation a
 * page shows brings them with their revision: a list newer than the one the page
 * holds replaces it, a count of another run of the backend included.
 *
 * - A list older than the one it holds is dropped.
 * - A decision shows at once and is not read back. One refused goes back to the
 *   requests as they were, and another page's decision reaches this one through the
 *   metadata.
 */
export const usePermissions = defineStore('permissions', () => {
  const product = useProduct()
  const conversations = useConversations()
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
    state.requests = answer.requests.map(view)
    state.error = null
  }

  /**
   * The requests each followed conversation's metadata brought last, which
   * were taken under the run the page held as they came: the same value
   * seen again, as after the page's next snapshot names another run, is not
   * taken again.
   */
  const brought = new Map<string, ConversationPermissions>()

  /** Takes `permissions`, unless it is the value of the conversation's metadata taken last. */
  function bring(conversationId: string, permissions: ConversationPermissions): void {
    if (brought.get(conversationId) !== permissions) {
      brought.set(conversationId, permissions)
      take(stateFor(conversationId), permissions)
    }
  }

  /** Takes the requests the conversation's metadata brings, once a page shows it, and each newer list it brings from then on. */
  function follow(conversationId: string): void {
    stateFor(conversationId)
    const metadata = conversations.metadataOf(conversationId)
    if (metadata) {
      bring(conversationId, metadata.permissions)
    }
  }

  watch(
    () => [...states.keys()].map((id) => [id, conversations.metadataOf(id)?.permissions] as const),
    (followed) => {
      for (const [id, permissions] of followed) {
        if (permissions) {
          bring(id, permissions)
        }
      }
    },
    { flush: 'sync' },
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
        // Another page decided it first, or the credential was removed since: the metadata brings the outcome.
        if (error.code === 'permission_request_not_found' || error.code === 'credential_removed') {
          return
        }
        // The values do not suit the request: said under the form, which keeps what was typed.
        if (error.code === 'invalid_answer' || error.code === 'credential_fields_differ') {
          // A value the request refused, which the user corrects in the card.
          state.error = formError(error, { invalid_answer: answerProblem(error) })
          return
        }
      }
      reportError(t('failures.answerRequest'), error, { userVisible: true })
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
