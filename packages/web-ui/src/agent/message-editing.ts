import { toRaw } from 'vue'
import { EditRejectedError, isEditableUserMessage } from '@demicodes/agent/client'
import type { EditRequest, TranscriptVersion } from '@demicodes/agent/client'
import type { Block, UserContentBlock } from '@demicodes/core'
import type { BlobReferenceSource } from './media-source'

export { EditRejectedError } from '@demicodes/agent/client'

/** Displayed media may be an authenticated blob reference supplied by the host. */
export type MessageEditContent = Extract<UserContentBlock, { type: 'text' | 'reference' }> | {
  [Kind in 'image' | 'video' | 'document']: {
    type: Kind
    source: Extract<UserContentBlock, { type: Kind }>['source'] | BlobReferenceSource
  }
}['image' | 'video' | 'document']

export interface MessageEditRequest extends Omit<EditRequest, 'content'> {
  content: MessageEditContent[]
}

export interface MessageEditState {
  phase: 'editing' | 'sending' | 'uncertain'
  request: MessageEditRequest
  error: string | null
}

export function beginMessageEdit(block: Block, version: TranscriptVersion): MessageEditState {
  if (!isEditableUserMessage(block)) {
    throw new Error('This message cannot be edited')
  }
  const content = structuredClone(toRaw(block.content))
  if (!content.some((part) => part.type === 'text')) {
    content.push({ type: 'text', text: '' })
  }
  return {
    phase: 'editing',
    request: {
      operationId: crypto.randomUUID(),
      targetBlockId: block.id,
      version: { ...version },
      content,
    },
    error: null,
  }
}

export function editHasContent(state: MessageEditState): boolean {
  return state.request.content.some((part) => part.type !== 'text' || part.text.trim())
}

export function isMessageEditSubmitKey(
  event: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'isComposing'>,
): boolean {
  return event.key === 'Enter' && (event.metaKey || event.ctrlKey) && !event.isComposing
}

/** Keep each draft detached; Vue proxies must not enter persisted request data. */
export function changeMessageEditContent(
  state: MessageEditState,
  change: (content: MessageEditContent[]) => void,
): MessageEditState {
  if (state.phase !== 'editing') {
    return state
  }
  const next = structuredClone(toRaw(state))
  change(next.request.content)
  return next
}

/** Only confirmation or explicit rejection permits changing a submitted request. */
export async function submitMessageEdit(host: {
  get(): MessageEditState | null
  set(state: MessageEditState | null): void
  send(request: MessageEditRequest): Promise<void>
}): Promise<void> {
  const draft = host.get()
  if (!draft || draft.phase === 'sending' || !editHasContent(draft)) {
    return
  }
  const request = structuredClone(toRaw(draft.request))
  host.set({ phase: 'sending', request, error: null })
  try {
    await host.send(request)
    if (host.get()?.request.operationId === request.operationId) {
      host.set(null)
    }
  } catch (error) {
    if (host.get()?.request.operationId === request.operationId) {
      host.set({
        phase: error instanceof EditRejectedError ? 'editing' : 'uncertain',
        request,
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }
}

/** A page reload loses the in-flight response, not the submitted request. */
export function restoreMessageEdit(state: MessageEditState | null): MessageEditState | null {
  return state?.phase === 'sending'
    ? { ...state, phase: 'uncertain', error: 'Confirmation was interrupted. Retry to check whether the edit was accepted.' }
    : state
}
