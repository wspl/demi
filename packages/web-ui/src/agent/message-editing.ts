import { inject, provide, type InjectionKey } from 'vue'
import { z } from 'zod'
import { EditRejectedError } from '@demicodes/conversation-client'
import {
  editRequestSchema,
  userContentBlockSchema,
  type Block,
  type ClientContent,
  type EditRequest,
  type SessionPhase,
  type Version,
} from '@demicodes/protocol'
import { reportError } from '../infra/errors'
import { plainCopy } from '../infra/plain-copy'
import { t } from './locales/text'
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
 * A file on a paired device the edit added, such as one a pasted message
 * held: the frame names it by its device and path, and the capsule shows the
 * device's name.
 */
export const editRemoteFileSchema = z.object({
  type: z.literal('remote_file'),
  deviceId: z.string().min(1),
  path: z.string().min(1),
  host: z.string(),
})

/**
 * One part of an edit: what the edited message holds, as the transcript shows
 * it, or a file the edit added.
 */
export const messageEditContentSchema = z.discriminatedUnion('type', [
  ...userContentBlockSchema.options,
  editUploadSchema,
  editRemoteFileSchema,
])
export type MessageEditContent = z.infer<typeof messageEditContentSchema>

export const messageEditRequestSchema = editRequestSchema.omit({ content: true }).extend({
  content: z.array(messageEditContentSchema),
})
export type MessageEditRequest = z.infer<typeof messageEditRequestSchema>

/**
 * An edit of a sent message in progress, in one of these phases:
 *
 * - `sending`: an edit the user sent from the editor.
 * - `regenerating`: a Regenerate sent without opening it, which `regenerate`
 *   marks while it is uncertain too, so Retry sends the same operation again.
 * - `uncertain`: the server's answer was lost. Retry checks the outcome before
 *   resending.
 *
 * A failure is told by a toast at the moment it happens; the state itself
 * carries no message.
 */
export const sentMessageEditSchema = z.object({
  kind: z.literal('message'),
  phase: z.enum(['editing', 'sending', 'regenerating', 'uncertain']),
  request: messageEditRequestSchema,
  regenerate: z.literal(true).optional(),
})
export type SentMessageEdit = z.infer<typeof sentMessageEditSchema>

/**
 * An edit of the message saved for later `savedId`, opened as `editId`. Save
 * writes its content back to the message in its place and Send sends it, each
 * ending it at once, so it is only ever being edited.
 */
export const savedMessageEditSchema = z.object({
  kind: z.literal('saved'),
  phase: z.literal('editing'),
  savedId: z.string().min(1),
  editId: z.string().min(1),
  content: z.array(messageEditContentSchema),
})
export type SavedMessageEdit = z.infer<typeof savedMessageEditSchema>

/** The edit the composer shows: of a sent message, or of a saved one. */
export const messageEditStateSchema = z.discriminatedUnion('kind', [sentMessageEditSchema, savedMessageEditSchema])
export type MessageEditState = z.infer<typeof messageEditStateSchema>

/** What names the edit's opening of the editor, new for each. */
export function editKey(state: MessageEditState): string {
  return state.kind === 'message' ? state.request.operationId : state.editId
}

/** What the edited message holds now. */
export function editContent(state: MessageEditState): MessageEditContent[] {
  return state.kind === 'message' ? state.request.content : state.content
}

/**
 * The edit of a message with its content as it was sent. Only a `user` block
 * is a message the user can edit.
 */
export function unchangedEditRequest(block: Block, version: Version): MessageEditRequest {
  if (block.type !== 'user') {
    throw new Error('This message cannot be edited')
  }
  return {
    operationId: crypto.randomUUID(),
    targetBlockId: block.id,
    version: { ...version },
    content: plainCopy(block.content),
  }
}

/**
 * Opens the editor on a message; one without text gets an empty line to type
 * in.
 */
export function beginMessageEdit(block: Block, version: Version): SentMessageEdit {
  const request = unchangedEditRequest(block, version)
  if (!request.content.some((part) => part.type === 'text')) {
    request.content.push({ type: 'text', text: '' })
  }
  return { kind: 'message', phase: 'editing', request }
}

/**
 * Opens the editor on the message saved for later `savedId`, holding
 * `content`; one without text gets an empty line to type in.
 */
export function beginSavedEdit(savedId: string, content: MessageEditContent[]): SavedMessageEdit {
  const opened = plainCopy(content)
  if (!opened.some((part) => part.type === 'text')) {
    opened.push({ type: 'text', text: '' })
  }
  return { kind: 'saved', phase: 'editing', savedId, editId: crypto.randomUUID(), content: opened }
}

export function editHasContent(state: MessageEditState): boolean {
  return editContent(state).some((part) => part.type !== 'text' || part.text.trim())
}

/** The web app exposes editing only for the latest explicit user submission. */
export function lastEditableUserMessageId(blocks: readonly MessageListBlock[]): string | null {
  return blocks.findLast((block) => block.type === 'user')?.id ?? null
}

/**
 * The message the page offers to edit, and to regenerate the answer of: the
 * last `user` block, while the conversation is idle and nothing waits to be
 * sent; null when none is offered.
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
 * it, and is absent otherwise, when the key moves the caret as usual.
 */
export function provideEditLastMessage(handler: () => (() => void) | undefined): void {
  provide(editLastMessageKey, handler)
}

export function useEditLastMessage(): () => (() => void) | undefined {
  return inject(editLastMessageKey, () => undefined)
}

/**
 * The edit as the conversation socket carries it: a file the message already
 * holds by its record's path and its media by blob reference, a file the edit
 * added by its upload, and no bytes at all. Media that is not a blob of the
 * conversation cannot be kept, and the edit is refused before it is sent.
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
      return { type: 'attachment', id: part.id }
    case 'upload':
      return { type: 'upload', ref: part.ref, fileName: part.fileName }
    case 'remote_file':
      return { type: 'remote_file', deviceId: part.deviceId, path: part.path }
    case 'image':
    case 'video':
      if (part.source.type !== 'ref') {
        throw new EditRejectedError({
          code: 'edit_media_not_stored',
          reason: `The message’s ${part.type} is not stored with the conversation, so an edit cannot keep it`,
        })
      }
      return { type: 'media', media: { type: part.type, ref: part.source.ref, mediaType: part.source.mediaType } }
    case 'document':
      return {
        type: 'media',
        media: { type: 'document', ref: part.source.ref, mediaType: part.source.mediaType, fileName: part.source.fileName },
      }
  }
}

/**
 * The accepted suffix stays visible but inactive until the edit is resolved.
 */
export function messageEditSuffixIds(
  blocks: readonly { id: string }[],
  targetBlockId: string | undefined,
): Set<string> {
  const index = blocks.findIndex((block) => block.id === targetBlockId)
  return new Set(index < 0 ? [] : blocks.slice(index).map((block) => block.id))
}

/**
 * Keep each draft detached; Vue proxies must not enter persisted request data.
 */
export function changeMessageEditContent<State extends MessageEditState>(
  state: State,
  change: (content: MessageEditContent[]) => void,
): State {
  if (state.phase !== 'editing') {
    return state
  }
  const next = plainCopy(state)
  change(editContent(next))
  return next
}

/**
 * Where an edit is kept while it is sent, and how the host sends it: as an
 * edit, or, `regenerate`, as a Regenerate of the answer to its message,
 * which sends no content.
 */
export interface MessageEditHost {
  get(): MessageEditState | null
  set(state: SentMessageEdit | null): void
  send(request: MessageEditRequest, regenerate: boolean): Promise<void>
}

/**
 * Only confirmation or explicit rejection permits changing a submitted request.
 */
export async function submitMessageEdit(host: MessageEditHost): Promise<void> {
  const draft = host.get()
  if (draft?.kind !== 'message' || draft.phase === 'sending' || draft.phase === 'regenerating' || !editHasContent(draft)) {
    return
  }
  const regenerate = draft.phase === 'uncertain' && draft.regenerate === true
  await sendMessageEdit(host, plainCopy(draft.request), regenerate ? 'regenerating' : 'sending')
}

/**
 * Regenerate: asks at once, without the editor, for a new answer to the
 * last message, `request` naming it with its content unchanged. It goes
 * as an edit does: a lost answer leaves it uncertain, for Retry in the
 * editor, and a refusal opens the editor on the message. Nothing is sent
 * while another edit is in progress.
 */
export async function regenerateMessage(host: MessageEditHost, request: MessageEditRequest): Promise<void> {
  if (host.get()) {
    return
  }
  await sendMessageEdit(host, plainCopy(request), 'regenerating')
}

async function sendMessageEdit(
  host: MessageEditHost,
  request: MessageEditRequest,
  phase: 'sending' | 'regenerating',
): Promise<void> {
  const regenerate = phase === 'regenerating'
  const current = () => {
    const state = host.get()
    return state?.kind === 'message' && state.request.operationId === request.operationId
  }
  host.set(regenerate ? { kind: 'message', phase, request, regenerate } : { kind: 'message', phase, request })
  try {
    await host.send(request, regenerate)
    if (current()) {
      host.set(null)
    }
  } catch (error) {
    const rejected = error instanceof EditRejectedError
    if (current()) {
      // A refused Regenerate opens the editor on the message, whose Save is an
      // edit.
      host.set(rejected || !regenerate
        ? { kind: 'message', phase: rejected ? 'editing' : 'uncertain', request }
        : { kind: 'message', phase: 'uncertain', request, regenerate })
    }
    reportError(
      rejected ? t('transcript.edit.rejected') : t('transcript.edit.unconfirmed'),
      error,
      { userVisible: true, expected: rejected },
    )
  }
}

/** A page reload loses the in-flight response, not the submitted request. */
export function restoreMessageEdit(state: MessageEditState | null): MessageEditState | null {
  return state?.kind === 'message' && (state.phase === 'sending' || state.phase === 'regenerating')
    ? { ...state, phase: 'uncertain' }
    : state
}
