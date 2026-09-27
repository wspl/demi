import type { ClientContent } from '@demicodes/protocol'
import { SerialQueue } from '@demicodes/utils'
import { isComposerFile } from '@demicodes/web-ui/agent/message-input/attachments'
import { ATTACHMENT_MARK } from '@demicodes/web-ui/markdown/user-markdown'
import { ApiError } from '../api/client'
import { changeReplacedDraft, loadDraft, saveDraft } from '../api/drafts'
import type { ConversationDraft, DraftFile, ReplacedAction } from '../api/generated/web-api'
import type { Conversation, ProductAttachment } from '../state/types'

// A conversation's draft kept by the backend and followed by every page
// (`web-application.md` § Drafts): the page saves what its user types, based
// on the revision its text was built on, and shows each newer draft it reads
// unless its user has a change the backend has not confirmed.

/** How long typing pauses before the page saves the draft. */
export const DRAFT_SAVE_DELAY_MS = 500

/** How long a save that did not reach the backend waits before it is tried again. */
export const DRAFT_RETRY_MS = 3_000

/** A draft as a save sends it: its text and each file as a frame names it. */
export interface DraftContent {
  text: string
  files: ClientContent[]
}

/**
 * The composer's draft as a save sends it. A file whose upload is not done
 * has no upload to name yet, so the text leaves out its mark until it has;
 * `complete` says whether every file of the message is in.
 */
export function draftContent(
  conversation: Pick<Conversation, 'draft' | 'files' | 'attachmentIds'>,
): { content: DraftContent; complete: boolean } {
  const parts = conversation.draft.split(ATTACHMENT_MARK)
  let text = parts[0] ?? ''
  const files: ClientContent[] = []
  let complete = true
  parts.slice(1).forEach((part, index) => {
    const id = conversation.attachmentIds[index]
    const file = conversation.files.find((item) => item.id === id)
    const named = file ? fileContent(file) : null
    if (named) {
      text += ATTACHMENT_MARK
      files.push(named)
    } else {
      complete = false
    }
    text += part
  })
  return { content: { text, files }, complete }
}

/** A file as a frame names it; none while its upload is not done. */
function fileContent(file: ProductAttachment): ClientContent | null {
  if (!isComposerFile(file)) {
    return { type: 'remote_file', deviceId: file.deviceId, path: file.path }
  }
  return file.upload ? { type: 'upload', ref: file.upload.id, fileName: file.name } : null
}

/** A draft's file as a save names it. */
function namedFile(file: DraftFile): ClientContent {
  return file.type === 'upload'
    ? { type: 'upload', ref: file.ref, fileName: file.fileName }
    : { type: 'remote_file', deviceId: file.deviceId, path: file.path }
}

/** Whether `content` is what `draft` holds. */
export function sameContent(content: DraftContent, draft: Pick<ConversationDraft, 'text' | 'files'>): boolean {
  return content.text === draft.text &&
    JSON.stringify(content.files) === JSON.stringify(draft.files.map(namedFile))
}

/**
 * Whether the page has a change the backend has not confirmed: a send not
 * yet accepted, whose emptied composer waits for it; a file still
 * uploading; or a composer that differs from the draft the page last read,
 * or was built on another revision of it.
 */
export function hasUnsavedDraft(conversation: Conversation): boolean {
  const { content, complete } = draftContent(conversation)
  if (conversation.pendingSend || !complete) {
    return true
  }
  const held = conversation.savedDraft
  if (!held) {
    return conversation.draftBase !== null || content.text !== '' || content.files.length > 0
  }
  return conversation.draftBase !== held.revision || !sameContent(content, held)
}

/** Whether a save has something to send now; a send not yet accepted holds it back. */
function needsSave(conversation: Conversation): boolean {
  const held = conversation.savedDraft
  if (conversation.persistence !== 'synced' || conversation.archived || conversation.pendingSend || !held) {
    return false
  }
  return conversation.draftBase !== held.revision || !sameContent(draftContent(conversation).content, held)
}

export function createDraftSync(options: {
  /** Shows `draft` in the conversation's composer, with its files. */
  apply: (conversation: Conversation, draft: ConversationDraft) => void
  report: (title: string, error: unknown) => void
}) {
  let lifetime = new AbortController()
  const timers = new Map<string, ReturnType<typeof setTimeout>>()
  const queues = new Map<string, SerialQueue>()
  /** Saves in flight, by conversation: a save on close waits for none. */
  const saving = new Set<string>()
  /** What each conversation's refused save sent: only another draft is tried again. */
  const refused = new Map<string, string>()

  function queue(id: string): SerialQueue {
    let current = queues.get(id)
    if (!current) {
      current = new SerialQueue()
      queues.set(id, current)
    }
    return current
  }

  function clearTimer(id: string): void {
    clearTimeout(timers.get(id))
    timers.delete(id)
  }

  /** Saves the draft once `delay` passes without another change, unless the backend refused this very draft. */
  function schedule(conversation: Conversation, delay: number): void {
    clearTimer(conversation.id)
    const refusedDraft = refused.get(conversation.id)
    if (!needsSave(conversation) || (refusedDraft !== undefined && refusedDraft === sent(conversation))) {
      return
    }
    timers.set(conversation.id, setTimeout(() => {
      timers.delete(conversation.id)
      void save(conversation)
    }, delay))
  }

  /** What a save of the conversation's draft would send, to tell one draft from another. */
  function sent(conversation: Conversation): string {
    return JSON.stringify(draftContent(conversation).content)
  }

  /** The page changed something of the conversation's: its draft is saved once typing pauses. */
  function changed(conversation: Conversation): void {
    schedule(conversation, DRAFT_SAVE_DELAY_MS)
  }

  /** Saves the draft now, after the conversation's requests before it. */
  function save(conversation: Conversation): Promise<void> {
    clearTimer(conversation.id)
    return queue(conversation.id).run(() => saveNow(conversation))
  }

  async function saveNow(conversation: Conversation, keepalive = false): Promise<void> {
    if (!needsSave(conversation)) {
      return
    }
    const signal = lifetime.signal
    const { content } = draftContent(conversation)
    saving.add(conversation.id)
    try {
      const draft = await saveDraft(
        conversation.id,
        { base: conversation.draftBase ?? 0, ...content },
        { signal, keepalive },
      )
      signal.throwIfAborted()
      // What the user typed since the request is a change on top of it.
      conversation.savedDraft = draft
      conversation.draftBase = draft.revision
    } catch (error) {
      if (signal.aborted) {
        return
      }
      if (error instanceof ApiError && error.status >= 400 && error.status < 500) {
        refused.set(conversation.id, JSON.stringify(content))
        options.report('Could not save the draft', error)
        return
      }
      // The save did not reach the backend, or the backend failed it.
      schedule(conversation, DRAFT_RETRY_MS)
    } finally {
      saving.delete(conversation.id)
    }
  }

  /**
   * Takes a draft read from the backend when it is newer than the one the
   * page holds: the composer shows it unless its user has an unsaved change,
   * whose save, built on the older revision, then wins.
   */
  function receive(conversation: Conversation, draft: ConversationDraft): void {
    const held = conversation.savedDraft
    if (held && draft.revision <= held.revision) {
      return
    }
    const unsaved = hasUnsavedDraft(conversation)
    const { content, complete } = draftContent(conversation)
    conversation.savedDraft = draft
    if (!conversation.pendingSend && complete && sameContent(content, draft)) {
      // The composer holds this draft already, whichever page saved it.
      conversation.draftBase = draft.revision
    } else if (!unsaved) {
      options.apply(conversation, draft)
      conversation.draftBase = draft.revision
    } else if (conversation.draftBase === null) {
      // Typed before any draft was read: built on none of them.
      conversation.draftBase = 0
    }
  }

  /**
   * Reads the draft, after the conversation's requests before it. A change
   * the backend has not confirmed, such as one this page restored from the
   * browser or kept while the conversation was archived, is saved after it.
   */
  function read(conversation: Conversation): Promise<void> {
    const signal = lifetime.signal
    return queue(conversation.id).run(async () => {
      const draft = await loadDraft(conversation.id, signal)
      signal.throwIfAborted()
      receive(conversation, draft)
      schedule(conversation, DRAFT_SAVE_DELAY_MS)
    })
  }

  /**
   * Restores or dismisses the replaced version the composer offers, after
   * saving what the composer holds. A restore shows the restored draft; a
   * replaced version that changed meanwhile is read again, so the composer
   * offers the one there is.
   */
  function act(conversation: Conversation, action: ReplacedAction): Promise<void> {
    const signal = lifetime.signal
    clearTimer(conversation.id)
    return queue(conversation.id).run(async () => {
      await saveNow(conversation)
      const replaced = conversation.savedDraft?.replaced
      if (!replaced) {
        return
      }
      try {
        const draft = await changeReplacedDraft(conversation.id, { action, revision: replaced.revision }, signal)
        signal.throwIfAborted()
        conversation.savedDraft = draft
        conversation.draftBase = draft.revision
        if (action === 'restore') {
          options.apply(conversation, draft)
        }
      } catch (error) {
        if (!(error instanceof ApiError && error.code === 'draft_changed')) {
          throw error
        }
        const draft = await loadDraft(conversation.id, signal)
        signal.throwIfAborted()
        receive(conversation, draft)
        schedule(conversation, DRAFT_SAVE_DELAY_MS)
      }
    })
  }

  /**
   * Saves every draft whose save waits for typing to pause: when the page is
   * hidden, or, with `keepalive`, when it closes. A save on close is sent at
   * once and never after another save of the conversation still in flight,
   * which could overtake it; IndexedDB keeps the change until the page opens
   * again.
   */
  function flush(conversations: readonly Conversation[], keepalive = false): void {
    for (const conversation of conversations) {
      if (!timers.has(conversation.id)) {
        continue
      }
      clearTimer(conversation.id)
      if (!keepalive) {
        void save(conversation)
      } else if (!saving.has(conversation.id)) {
        void saveNow(conversation, true)
      }
    }
  }

  function stop(): void {
    lifetime.abort()
    lifetime = new AbortController()
    for (const id of [...timers.keys()]) {
      clearTimer(id)
    }
    queues.clear()
    saving.clear()
    refused.clear()
  }

  return { changed, save, read, act, flush, stop }
}
