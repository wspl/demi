import { inject, provide, toRaw, type InjectionKey } from 'vue'
import { z } from 'zod'
import { EditRejectedError } from '@demicodes/conversation-client'
import {
  editRequestSchema,
  userContentBlockSchema,
  type Block,
  type ClientContent,
  type EditRequest,
  type SessionPhase,
  type TranscriptVersion,
} from '@demicodes/protocol'
import { reportError } from '../infra/errors'
import type { MessageListBlock } from './pending-steers'

export { EditRejectedError } from '@demicodes/conversation-client'

/**
 * A file the edit added, once its upload is done: the upload the edit names
 * it by, and what the backend answered about it, which its capsule shows.
 */
export const editUploadSchema = z.object({
  type: z.literal('upload'),
  /** The upload's attachment id. */
  ref: z.string().min(1),
  fileName: z.string().min(1),
  mediaType: z.string(),
  /** The bytes in the user's blobs, where the capsule's picture loads from. */
  sha256: z.string(),
  snippet: z.string().optional(),
})
export type EditUpload = z.infer<typeof editUploadSchema>

/**
 * One part of an edit: what the edited message holds, as the transcript shows
 * it, or a file the edit added.
 */
export const messageEditContentSchema = z.discriminatedUnion('type', [
  ...userContentBlockSchema.options,
  editUploadSchema,
])
export type MessageEditContent = z.infer<typeof messageEditContentSchema>

export const messageEditRequestSchema = editRequestSchema.omit({ content: true }).extend({
  content: z.array(messageEditContentSchema),
})
export type MessageEditRequest = z.infer<typeof messageEditRequestSchema>

/**
 * An edit in progress. `sending` is an edit the user sent from the editor,
 * `regenerating` one Regenerate sent without opening it. `uncertain` means
 * the server's answer was lost: Retry checks the outcome before resending. A
 * failure is told by a toast at the moment it happens; the state itself
 * carries no message.
 */
export const messageEditStateSchema = z.object({
  phase: z.enum(['editing', 'sending', 'regenerating', 'uncertain']),
  request: messageEditRequestSchema,
})
export type MessageEditState = z.infer<typeof messageEditStateSchema>

/**
 * The edit of a message with its content as it was sent. Only a `user` block
 * is a message the user can edit (`message-editing.md` § Editable blocks).
 */
export function unchangedEditRequest(block: Block, version: TranscriptVersion): MessageEditRequest {
  if (block.type !== 'user') {
    throw new Error('This message cannot be edited')
  }
  return {
    operationId: crypto.randomUUID(),
    targetBlockId: block.id,
    version: { ...version },
    content: structuredClone(toRaw(block.content)),
  }
}

/** Opens the editor on a message; one without text gets an empty line to type in. */
export function beginMessageEdit(block: Block, version: TranscriptVersion): MessageEditState {
  const request = unchangedEditRequest(block, version)
  if (!request.content.some((part) => part.type === 'text')) {
    request.content.push({ type: 'text', text: '' })
  }
  return { phase: 'editing', request }
}

export function editHasContent(state: MessageEditState): boolean {
  return state.request.content.some((part) => part.type !== 'text' || part.text.trim())
}

/** The web app exposes editing only for the latest explicit user submission. */
export function lastEditableUserMessageId(blocks: readonly MessageListBlock[]): string | null {
  return blocks.findLast((block) => block.type === 'user')?.id ?? null
}

/**
 * The message the page offers to edit, and to regenerate the answer of
 * (`message-editing.md` § Regenerate and the Up Arrow key): the last `user`
 * block, while the conversation is idle and nothing waits to be sent; null
 * when none is offered.
 */
export function offeredEditId(conversation: {
  blocks: readonly MessageListBlock[]
  phase: SessionPhase
  queue: readonly unknown[]
  pendingSteers: readonly unknown[]
}): string | null {
  if (conversation.phase !== 'idle' || conversation.queue.length || conversation.pendingSteers.length) {
    return null
  }
  return lastEditableUserMessageId(conversation.blocks)
}

const editLastMessageKey: InjectionKey<() => (() => void) | undefined> = Symbol('edit-last-message')

/**
 * What the main composer's Up Arrow does in an empty composer, read at each
 * press: opens the editor on the last message while the page offers editing
 * it, and is absent otherwise, when the key moves the caret as usual
 * (`message-editing.md` § Regenerate and the Up Arrow key).
 */
export function provideEditLastMessage(handler: () => (() => void) | undefined): void {
  provide(editLastMessageKey, handler)
}

export function useEditLastMessage(): () => (() => void) | undefined {
  return inject(editLastMessageKey, () => undefined)
}

/**
 * The edit as the conversation socket carries it (`message-editing.md`
 * § Files the edit keeps): a file the message already holds by its record's
 * path and its media by blob reference, a file the edit added by its upload,
 * and no bytes at all. Media that is not a blob of the conversation cannot be
 * kept, and the edit is refused before it is sent.
 */
export function sentEditRequest(request: MessageEditRequest): EditRequest {
  return { ...request, content: request.content.map(sentEditContent) }
}

function sentEditContent(part: MessageEditContent): ClientContent {
  switch (part.type) {
    case 'text':
      return { type: 'text', text: part.text }
    case 'reference':
      return { type: 'reference', reference: part.reference }
    case 'attachment':
      return { type: 'attachment', path: part.path }
    case 'upload':
      return { type: 'upload', ref: part.ref, fileName: part.fileName }
    case 'image':
    case 'video':
      if (part.source.type !== 'ref') {
        throw new EditRejectedError(`The message’s ${part.type} is not stored with the conversation, so an edit cannot keep it`)
      }
      return { type: 'media', media: { type: part.type, ref: part.source.ref, mediaType: part.source.mediaType } }
    case 'document':
      return {
        type: 'media',
        media: { type: 'document', ref: part.source.ref, mediaType: part.source.mediaType, fileName: part.source.fileName },
      }
  }
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

/** Where an edit is kept while it is sent, and how the host sends it. */
export interface MessageEditHost {
  get(): MessageEditState | null
  set(state: MessageEditState | null): void
  send(request: MessageEditRequest): Promise<void>
}

/** Only confirmation or explicit rejection permits changing a submitted request. */
export async function submitMessageEdit(host: MessageEditHost): Promise<void> {
  const draft = host.get()
  if (!draft || draft.phase === 'sending' || draft.phase === 'regenerating' || !editHasContent(draft)) {
    return
  }
  await sendMessageEdit(host, structuredClone(toRaw(draft.request)), 'sending')
}

/**
 * Regenerate: sends the edit of the last message with its content unchanged
 * at once, without the editor (`message-editing.md` § Regenerate and the Up
 * Arrow key). It is an edit like any other: a lost answer leaves it
 * uncertain, for Retry in the editor, and a refusal opens the editor on it.
 * Nothing is sent while another edit is in progress.
 */
export async function regenerateMessage(host: MessageEditHost, request: MessageEditRequest): Promise<void> {
  if (host.get()) {
    return
  }
  await sendMessageEdit(host, structuredClone(toRaw(request)), 'regenerating')
}

async function sendMessageEdit(
  host: MessageEditHost,
  request: MessageEditRequest,
  phase: 'sending' | 'regenerating',
): Promise<void> {
  host.set({ phase, request })
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
      rejected ? 'The edit was not accepted.' : 'Could Not Confirm the Edit',
      error,
      { userVisible: true, expected: rejected },
    )
  }
}

/** A page reload loses the in-flight response, not the submitted request. */
export function restoreMessageEdit(state: MessageEditState | null): MessageEditState | null {
  return state?.phase === 'sending' || state?.phase === 'regenerating' ? { ...state, phase: 'uncertain' } : state
}
