import { toRaw } from 'vue'
import { EditRejectedError, isEditableUserMessage } from '@demicodes/agent/client'
import type { EditRequest, TranscriptVersion } from '@demicodes/agent/client'
import type { Block, UserContentBlock } from '@demicodes/core'
import { reportError } from '../infra/errors'
import type { BlobReferenceSource } from './media-source'
import type { MessageListBlock } from './pending-steers'

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

/**
 * An edit in progress. `uncertain` means the server's answer was lost: Retry
 * checks the outcome before resending. A failure is told by a toast at the
 * moment it happens; the state itself carries no message.
 */
export interface MessageEditState {
  phase: 'editing' | 'sending' | 'uncertain'
  request: MessageEditRequest
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
  }
}

export function editHasContent(state: MessageEditState): boolean {
  return state.request.content.some((part) => part.type !== 'text' || part.text.trim())
}

/** The browser exposes editing only for the latest explicit user submission. */
export function lastEditableUserMessageId(blocks: readonly MessageListBlock[]): string | null {
  return blocks.findLast((block) => block.type === 'user' && isEditableUserMessage(block))?.id ?? null
}

/** The accepted suffix stays visible but inactive until the edit is resolved. */
export function messageEditSuffixIds(
  blocks: readonly { id: string }[],
  targetBlockId: string | undefined,
): Set<string> {
  const index = blocks.findIndex((block) => block.id === targetBlockId)
  return new Set(index < 0 ? [] : blocks.slice(index).map((block) => block.id))
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
  host.set({ phase: 'sending', request })
  try {
    await host.send(request)
    if (host.get()?.request.operationId === request.operationId) {
      host.set(null)
    }
  } catch (error) {
    const rejected = error instanceof EditRejectedError
    if (host.get()?.request.operationId === request.operationId) {
      host.set({ phase: rejected ? 'editing' : 'uncertain', request })
    }
    reportError(
      rejected ? 'The edit was not accepted' : 'Could not confirm the edit',
      error,
      { userVisible: true, expected: rejected },
    )
  }
}

/** A page reload loses the in-flight response, not the submitted request. */
export function restoreMessageEdit(state: MessageEditState | null): MessageEditState | null {
  return state?.phase === 'sending' ? { ...state, phase: 'uncertain' } : state
}
