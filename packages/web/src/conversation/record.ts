import { markRaw, type Raw } from 'vue'
import type { ClientContent, MessageComment } from '@demicodes/protocol'
import { SerialQueue } from '@demicodes/utils'
import type { HeadlineText } from '@demicodes/web-ui/ui/ui-text'
import { Slot, type SlotChange } from '@demicodes/web-ui/infra/slot'
import { isComposerFile } from '@demicodes/web-ui/agent/message-input/attachments'
import { ATTACHMENT_MARK } from '@demicodes/web-ui/markdown/user-markdown'
import type { MessageEditContent } from '@demicodes/web-ui/agent/message-editing'
import { splitMessageContent } from '@demicodes/web-ui/agent/message-input/message-content'
import { ApiError } from '../api/client'
import { OVER_ANY } from './drafts'
import { saveDraft } from '../api/drafts'
import { deleteSaved, postSaved, putSaved, saveFromDraft } from '../api/saved'
import type { ConversationDraft, DraftFile, ListedSavedMessage, Revisions, SavedListing } from '../api/generated/web-api'
import type { Conversation, ProductAttachment } from '../state/types'
import { t } from './locales/text'

// A conversation's draft and its messages saved for later, which the backend
// changes together, as the one slot the page holds of them:
//
// - Every write of either is a change of the slot, sent in the order made after
//   the writes before it. Its answer names the revisions it produced, so the
//   change leaves once a read of them is held.
// - The draft is the last one written, on whichever page. The composer shows
//   the slot's draft. What its user types is a change of it, sent half a
//   second after typing pauses. A newer draft read shows in it unless its user
//   has a change the backend has not confirmed.
// - Text the user wrote is never only in a change: a message a refused Save
//   for Later carried goes back to the composer, and so does a draft write's
//   text while no later change of the draft took its place. An edit of a saved
//   message that the backend refused is dropped.
// - The list holds each saved message as the composer lists it, its text's
//   opening and length; the page's own change lists the text it wrote whole.
//   The conversation's metadata brings the list under its revision.

/** How long typing pauses before the page saves the draft. */
export const DRAFT_SAVE_DELAY_MS = 500

/** How long a save that failed waits before it is tried again. */
export const DRAFT_RETRY_MS = 3_000

/** The draft and the saved messages as the page holds them. */
export interface ConversationRecord {
  draft: ConversationDraft
  saved: ListedSavedMessage[]
}

/** A message's text and files, as a saved message holds them. */
export interface Message {
  text: string
  files: DraftFile[]
}

/** What a draft holds: the message, and the comments waiting to be sent with it, which no saved message takes. */
export interface Writing extends Message {
  comments: MessageComment[]
}

/** What the page keeps with a change of the record. */
export interface RecordMeta {
  kind: 'typing' | 'save' | 'write' | 'delete' | 'resave' | 'send'
  /**
   * The user's writing the change carries, which a refusal must not lose:
   * it goes back to the composer. An edit of a saved message carries none.
   */
  carries: Writing | null
}

export type RecordChange = SlotChange<ConversationRecord, RecordMeta>
export type RecordSlot = Slot<ConversationRecord, RecordMeta>

/** A conversation's record as the page holds it before reading anything. */
export function newRecord(): Raw<RecordSlot> {
  const slot = new Slot<ConversationRecord, RecordMeta>()
  slot.replace({ draft: { revision: 0, text: '', files: [], comments: [] }, saved: [] })
  return markRaw(slot)
}

/**
 * The message an edit of a saved message holds: its Markdown, and each file
 * where its mark stands. A saved message's edit holds its files as uploads
 * and device files alone, as the message and the edit's own files are.
 */
export function editedMessage(content: readonly MessageEditContent[]): Message {
  const { markdown, files } = splitMessageContent(content)
  return {
    text: markdown,
    files: files.flat().map((block): DraftFile => {
      switch (block.type) {
        case 'upload':
          return {
            type: 'upload',
            ref: block.ref,
            fileName: block.fileName,
            mediaType: block.mediaType,
            sha256: block.sha256,
            ...(block.snippet ? { snippet: block.snippet } : {}),
          }
        case 'remote_file':
          return { type: 'remote_file', deviceId: block.deviceId, path: block.path }
        default:
          throw new Error(`A saved message holds no ${block.type} file`)
      }
    }),
  }
}

/** A draft's file, or a saved message's, as a save names it. */
function namedFile(file: DraftFile): ClientContent {
  return file.type === 'upload'
    ? { type: 'upload', ref: file.ref, fileName: file.fileName }
    : { type: 'remote_file', deviceId: file.deviceId, path: file.path }
}

/** Whether `a` and `b` hold the same text and files. */
function sameMessage(a: Message, b: Message): boolean {
  return a.text === b.text && JSON.stringify(a.files.map(namedFile)) === JSON.stringify(b.files.map(namedFile))
}

/** How many characters `text` holds, as a listing counts them: by code point. */
function characters(text: string): number {
  return [...text].length
}

/** The message `id` as the list shows it while the page's write of it is on its way: its whole text. */
function listedAs(id: string, message: Message): ListedSavedMessage {
  return { id, opening: message.text, length: characters(message.text), files: message.files, savedAt: new Date().toISOString() }
}

/** Whether `listed` lists `message`: the same opening of a text as long, and the same files. */
function lists(listed: ListedSavedMessage, message: Message): boolean {
  return listed.length === characters(message.text) && message.text.startsWith(listed.opening)
    && JSON.stringify(listed.files.map(namedFile)) === JSON.stringify(message.files.map(namedFile))
}

/** The whole text of `listed` when its listing holds it whole; null when it holds only the opening. */
export function wholeListed(listed: ListedSavedMessage): Message | null {
  return characters(listed.opening) === listed.length ? { text: listed.opening, files: listed.files } : null
}

/** Whether `a` and `b` hold the same text, files and comments. */
function sameWriting(a: Writing, b: Writing): boolean {
  return sameMessage(a, b) && JSON.stringify(a.comments) === JSON.stringify(b.comments)
}

/** A message with no text but blank space and no file, which no list keeps. */
function isBlankMessage(message: Message): boolean {
  return !message.text.trim() && message.files.length === 0
}

/** A draft with nothing to keep: a blank message and no comment. */
function isBlank(writing: Writing): boolean {
  return isBlankMessage(writing) && writing.comments.length === 0
}

/**
 * The composer's message as a save holds it. A file whose upload is not done
 * has no upload to name yet, so the text leaves out its mark until it has;
 * `complete` says whether every file of the message is in.
 */
export function composerWriting(
  conversation: Pick<Conversation, 'draft' | 'files' | 'attachmentIds' | 'comments'>,
): { writing: Writing; complete: boolean } {
  const parts = conversation.draft.split(ATTACHMENT_MARK)
  let text = parts[0] ?? ''
  const files: DraftFile[] = []
  let complete = true
  parts.slice(1).forEach((part, index) => {
    const id = conversation.attachmentIds[index]
    const file = conversation.files.find((item) => item.id === id)
    const named = file ? draftFile(file) : null
    if (named) {
      text += ATTACHMENT_MARK
      files.push(named)
    } else {
      complete = false
    }
    text += part
  })
  return { writing: { text, files, comments: [...conversation.comments] }, complete }
}

/** A composer's file as a draft holds it; none while its upload is not done. */
export function draftFile(file: ProductAttachment): DraftFile | null {
  if (!isComposerFile(file)) {
    return { type: 'remote_file', deviceId: file.deviceId, path: file.path }
  }
  return file.upload
    ? {
        type: 'upload',
        ref: file.upload.id,
        fileName: file.name,
        mediaType: file.upload.mediaType,
        sha256: file.upload.sha256,
        ...(file.snippet ? { snippet: file.snippet } : {}),
      }
    : null
}

/** `list` with `message` at the top, unless the list holds one of its id. */
function withSaved(list: ListedSavedMessage[], message: ListedSavedMessage): ListedSavedMessage[] {
  return list.some((each) => each.id === message.id) ? list : [message, ...list]
}

/** `record` without the saved message `id`; the same record when it has none. */
function withoutSaved(record: ConversationRecord, id: string): ConversationRecord {
  return record.saved.some((message) => message.id === id)
    ? { ...record, saved: record.saved.filter((message) => message.id !== id) }
    : record
}

/** Resolves after `ms`, or at once when `signal` aborts. */
function pause(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms)
    signal.addEventListener('abort', () => {
      clearTimeout(timer)
      resolve()
    }, { once: true })
  })
}

/** A refusal the backend answered, which sending again would not change. */
function isRefusal(error: unknown): boolean {
  return error instanceof ApiError && error.status >= 400 && error.status < 500
}

/** A failure of a write: the ID the log names it by (`ReportErrorOptions.site`) and its title. */
interface Failure {
  site: string
  title: HeadlineText
}

function failed(site: string, title: HeadlineText): Failure {
  return { site, title }
}

export function createRecordSync(options: {
  /** Shows `writing` in the conversation's composer, with its files. */
  show: (conversation: Conversation, writing: Writing) => void
  /**
   * Puts `message`, with its files, in the conversation's composer before what
   * it holds, files still uploading included, which `show` would not keep.
   */
  prepend: (conversation: Conversation, message: Message) => void
  /** Makes the conversation's backend record, when it has none yet. */
  persist: (conversation: Conversation) => Promise<void>
  report: (site: string, title: HeadlineText, error: unknown) => void
}) {
  let lifetime = new AbortController()
  const timers = new Map<string, ReturnType<typeof setTimeout>>()
  /** Each conversation's writes of its record, in order. */
  let queue = new SerialQueue()
  /** Each conversation's change of its draft that its user types and the page has not sent yet. */
  const typing = new Map<string, RecordChange>()
  /**
   * The conversations whose composer this web browser kept, restored before
   * their draft is read, with the revision of the backend's draft it was
   * written over (`OVER_ANY` for any).
   */
  const restored = new Map<string, number>()
  /** The changes that hide a saved message being sent, by the send's turn id. */
  const sends = new Map<string, RecordChange>()
  /**
   * What each composer held when the page last looked, as `held` names it:
   * only a composer that changed since was typed in; one that did not may
   * only not have followed a newer draft yet.
   */
  const seen = new Map<string, string>()

  /** A writing as `seen` keeps it. */
  function held(writing: Writing): string {
    return JSON.stringify([writing.text, writing.files.map(namedFile), writing.comments])
  }

  function clearTimer(id: string): void {
    clearTimeout(timers.get(id))
    timers.delete(id)
  }

  /** The draft the slot shows. */
  function shownDraft(conversation: Conversation): ConversationDraft {
    return conversation.record.shown.value!.draft
  }

  /**
   * The composer shows the slot's draft: a draft read, a change of the
   * page's, or a refusal, shows in it, unless a file of its message is still
   * uploading, which it keeps.
   */
  function follow(conversation: Conversation): void {
    if (conversation.persistence !== 'synced' && conversation.record.pending('draft').length === 0) {
      return
    }
    const { writing, complete } = composerWriting(conversation)
    const shown = shownDraft(conversation)
    if (!sameWriting(writing, shown)) {
      if (!complete) {
        return
      }
      seen.set(conversation.id, held(shown))
      options.show(conversation, shown)
    }
  }

  /**
   * Whether the page has a change the backend has not confirmed: one of the
   * draft not answered yet, a file still uploading, a send not yet accepted,
   * or a composer that does not show the slot's draft.
   */
  function unsaved(conversation: Conversation): boolean {
    const { writing, complete } = composerWriting(conversation)
    return !complete ||
      conversation.pendingSends.length > 0 ||
      // A Save for Later empties the draft in the backend with its own write,
      // which the page keeps in its queue; the emptied composer is not input.
      conversation.record.pending('draft').some((change) => change.status !== 'landed' && change.meta.kind !== 'save') ||
      !sameWriting(writing, shownDraft(conversation))
  }

  /**
   * The revision of the backend's draft the composer's text is written over:
   * the one the page last read or had answered, and while the draft is not
   * read yet, the one a restored composer was written over. This web browser
   * keeps it with an unconfirmed composer, and never sends it.
   */
  function over(conversation: Conversation): number {
    const read = conversation.record.revision('draft')
    return read >= 0 ? read : restored.get(conversation.id) ?? 0
  }

  /** A composer this web browser kept, written over the backend's draft at revision `over`, holds a change the backend has not accepted. */
  function restore(conversation: Conversation, over: number): void {
    restored.set(conversation.id, over)
  }

  /**
   * The page changed something of the conversation's: what its composer
   * holds becomes the change of the draft not sent yet, saved once typing
   * pauses. A composer that holds the slot's draft again has none.
   */
  function changed(conversation: Conversation): void {
    if (conversation.persistence !== 'synced' || conversation.archived) {
      return
    }
    const record = conversation.record
    const { writing } = composerWriting(conversation)
    const unsent = typing.get(conversation.id)
    // A composer this web browser kept, restored before the draft is read,
    // is a change whatever it holds, an empty one after a send included:
    // there is no draft yet to compare it
    // with, and the draft read compares with it, never the other way round.
    // The first look counts it so; the change it makes brings this back
    // before it is kept as the typing, and finds it seen.
    const unread = record.revision('draft') < 0 && restored.has(conversation.id)
    if (!(unread && !seen.has(conversation.id)) && (seen.get(conversation.id) ?? held({ text: '', files: [], comments: [] })) === held(writing)) {
      return
    }
    seen.set(conversation.id, held(writing))
    const below = unsent ? record.below(unsent)! : record.shown.value!
    if (!unread && sameWriting(writing, below.draft)) {
      if (unsent) {
        record.leave([unsent])
        typing.delete(conversation.id)
        clearTimer(conversation.id)
      }
      return
    }
    const carries = writing
    const apply = (value: ConversationRecord) => ({
      ...value,
      draft: { ...value.draft, text: writing.text, files: writing.files, comments: writing.comments },
    })
    if (unsent) {
      unsent.meta.carries = carries
      unsent.revise(apply)
    } else {
      const change = record.change(apply, {
        parts: ['draft'],
        meta: { kind: 'typing', carries },
      })
      typing.set(conversation.id, change)
    }
    schedule(conversation, DRAFT_SAVE_DELAY_MS)
  }

  /** Saves the draft once `delay` passes without another change. */
  function schedule(conversation: Conversation, delay: number): void {
    clearTimer(conversation.id)
    if (!typing.has(conversation.id)) {
      return
    }
    timers.set(conversation.id, setTimeout(() => {
      timers.delete(conversation.id)
      void save(conversation)
    }, delay))
  }

  /** Saves the draft now, after the conversation's writes before it. */
  function save(conversation: Conversation): Promise<void> {
    clearTimer(conversation.id)
    return queue.run(conversation.id, () => sendTyping(conversation))
  }

  /**
   * Sends the change its user typed, from inside the conversation's order.
   * A send not yet accepted holds it back: the page saves the emptied draft
   * once the backend has the message.
   */
  async function sendTyping(conversation: Conversation, keepalive = false): Promise<void> {
    const change = typing.get(conversation.id)
    if (!change || !conversation.record.pending().includes(change) || conversation.persistence !== 'synced' ||
      conversation.archived || conversation.pendingSends.length > 0) {
      return
    }
    // A restored composer waits for the draft to be read, which says whether
    // the backend took its text or holds a later one (`receiveDraft`).
    if (restored.has(conversation.id) && conversation.record.revision('draft') < 0) {
      return
    }
    typing.delete(conversation.id)
    clearTimer(conversation.id)
    const signal = lifetime.signal
    const writing = change.meta.carries!
    change.send()
    try {
      const answer = await saveDraft(
        conversation.id,
        { text: writing.text, files: writing.files.map(namedFile), comments: writing.comments },
        { signal, keepalive },
      )
      signal.throwIfAborted()
      landed(conversation, change, answer)
    } catch (error) {
      if (signal.aborted) {
        return
      }
      // A refused save is reported, and tried again with the next change;
      // one the backend failed is tried again a few seconds later.
      refused(conversation, change, isRefusal(error) ? failed('failures.saveDraft', t('failures.saveDraft')) : null, error)
      if (!isRefusal(error)) {
        schedule(conversation, DRAFT_RETRY_MS)
      }
    }
  }

  /** A write of `change` was answered: the draft it carries is read, and the change leaves once the revisions it names are held. */
  function landed(
    conversation: Conversation,
    change: RecordChange,
    answer: { draft?: ConversationDraft; revisions: Revisions },
  ): boolean {
    const { draft } = answer
    if (draft) {
      conversation.record.receive('draft', (value) => ({ ...value, draft }), { revision: answer.revisions.draft })
    }
    const left = conversation.record.land([change], { revisions: answer.revisions })
    follow(conversation)
    return left
  }

  /**
   * A write of `change` failed: the change leaves, the failure is reported
   * when `failure` names it, and what the user wrote in it is kept. A draft
   * write's text goes back to the composer while no later change of the draft
   * took its place; a message a Save for Later carried goes back before what
   * the composer holds now. An edit of a saved message carries nothing: the
   * backend refused it because the message was sent or deleted meanwhile, or
   * the edit itself, and the failure says which.
   */
  function refused(conversation: Conversation, change: RecordChange, failure: Failure | null, error: unknown): void {
    const record = conversation.record
    const carried = change.meta.carries
    record.drop([change])
    if (failure) {
      options.report(failure.site, failure.title, error)
    }
    if (carried && !isBlank(carried)) {
      const shown = record.shown.value!
      if (change.meta.kind === 'typing') {
        if (!sameWriting(shown.draft, carried) && !typing.has(conversation.id) && record.pending('draft').length === 0) {
          const again = record.change(
            (value) => ({ ...value, draft: { ...value.draft, text: carried.text, files: carried.files, comments: carried.comments } }),
            { parts: ['draft'], meta: { kind: 'typing', carries: carried } },
          )
          typing.set(conversation.id, again)
        }
      } else if (!shown.saved.some((message) => lists(message, carried))) {
        putBack(conversation, carried)
      }
    }
    follow(conversation)
  }

  /**
   * Puts the message a refused Save for Later carried back in the composer,
   * before what the composer holds now, which stays after it; the draft's
   * waiting comments stay as they are.
   */
  function putBack(conversation: Conversation, message: Message): void {
    const record = conversation.record
    const unsent = typing.get(conversation.id)
    const { writing: now, complete } = composerWriting(conversation)
    if (!complete) {
      // A file is still uploading, which the slot's draft does not hold: the
      // composer takes the message itself, and its change follows from it.
      options.prepend(conversation, message)
      return
    }
    const text = isBlankMessage(now) ? message.text : `${message.text}\n\n${now.text}`
    const merged: Writing = { text, files: [...message.files, ...now.files], comments: now.comments }
    const apply = (value: ConversationRecord) => ({
      ...value,
      draft: { ...value.draft, text: merged.text, files: merged.files, comments: merged.comments },
    })
    if (unsent) {
      unsent.meta.carries = merged
      unsent.revise(apply)
    } else {
      typing.set(conversation.id, record.change(apply, { parts: ['draft'], meta: { kind: 'typing', carries: merged } }))
    }
    schedule(conversation, DRAFT_SAVE_DELAY_MS)
  }

  /**
   * Writes `change` after the conversation's writes before it: `request`
   * answers what the write produced. `sending` runs as it goes out, for a
   * change that settles what it shows then. A refusal of the backend ends the
   * write (`refused`). With `retrying`, a failure of the backend or of the
   * network is reported once and the write is sent again every few seconds,
   * holding the writes after it, until it lands or is refused. Answers how it
   * ended.
   */
  function write(
    conversation: Conversation,
    change: RecordChange,
    failure: Failure,
    request: (signal: AbortSignal) => Promise<{ draft?: ConversationDraft; revisions: Revisions }>,
    sending?: () => void,
    retrying = false,
  ): Promise<'landed' | 'waiting' | 'refused'> {
    const signal = lifetime.signal
    return queue.run(conversation.id, async () => {
      if (!conversation.record.pending().includes(change)) {
        return 'refused' as const
      }
      sending?.()
      change.send()
      let reported = false
      for (;;) {
        try {
          const answer = await request(signal)
          signal.throwIfAborted()
          return landed(conversation, change, answer) ? 'landed' as const : 'waiting' as const
        } catch (error) {
          if (signal.aborted) {
            return 'refused' as const
          }
          if (retrying && !isRefusal(error)) {
            if (!reported) {
              reported = true
              options.report(failure.site, failure.title, error)
            }
            await pause(DRAFT_RETRY_MS, signal)
            continue
          }
          refused(conversation, change, failure, error)
          return 'refused' as const
        }
      }
    })
  }

  /**
   * Save for Later: `writing`, the composer's message, stands at the top of
   * the list under `id` and leaves the composer at once; the backend keeps
   * it and empties the draft with it, after the writes before it, and the
   * save is sent again until the backend answers it. With `again`, the save
   * is one the page kept from before, whose message is no longer the
   * composer's: the composer is left as it is.
   */
  function saveForLater(conversation: Conversation, id: string, writing: Message, again = false): Promise<'landed' | 'waiting' | 'refused'> {
    const record = conversation.record
    if (again) {
      const message = listedAs(id, writing)
      const change = record.change((value) => ({ ...value, saved: withSaved(value.saved, message) }), {
        parts: ['saved'],
        meta: { kind: 'save', carries: { ...writing, comments: [] } },
      })
      return sendSave(conversation, change, id, writing)
    }
    // The message is the composer's: the change its user typed is in it.
    const unsent = typing.get(conversation.id)
    // The comments the change its user typed holds and the writes below it
    // do not, such as one saved while an earlier save is on its way: the
    // backend keeps the draft's comments as it has them, so these follow
    // the save in a save of the emptied draft of their own.
    const comments = unsent?.meta.carries?.comments
    const unsaved = unsent && comments && JSON.stringify(comments) !== JSON.stringify(record.below(unsent)!.draft.comments)
    if (unsent) {
      record.leave([unsent])
      typing.delete(conversation.id)
      clearTimer(conversation.id)
    }
    const message = listedAs(id, writing)
    // The draft's comments stay waiting there, as the backend keeps them.
    const change = record.change(
      (value) => ({ draft: { ...value.draft, text: '', files: [] }, saved: withSaved(value.saved, message) }),
      {
        parts: ['draft', 'saved'],
        meta: { kind: 'save', carries: { ...writing, comments: [] } },
      },
    )
    const kept = unsaved
      ? record.change(
        (value) => ({ ...value, draft: { ...value.draft, text: '', files: [], comments } }),
        {
          parts: ['draft'],
          meta: { kind: 'typing', carries: { text: '', files: [], comments } },
        },
      )
      : null
    follow(conversation)
    const saved = sendSave(conversation, change, id, writing)
    if (kept) {
      // Saved once the save has gone, which it is built on; a change typed
      // meanwhile carries the comments the composer shows, these among them.
      void saved.then(() => {
        if (!record.pending().includes(kept)) {
          return
        }
        if (typing.has(conversation.id)) {
          record.leave([kept])
          return
        }
        typing.set(conversation.id, kept)
        void save(conversation)
      })
    }
    return saved
  }

  /** The write of `change`, the Save for Later of `writing` under `id`. */
  function sendSave(conversation: Conversation, change: RecordChange, id: string, writing: Message): Promise<'landed' | 'waiting' | 'refused'> {
    return write(conversation, change, failed('failures.saveMessage', t('failures.saveMessage')), async (signal) => {
      await options.persist(conversation)
      return saveFromDraft(
        conversation.id,
        { id, text: writing.text, files: writing.files.map(namedFile) },
        signal,
      )
    }, undefined, true)
  }

  /**
   * Save of an edit of the saved message `id`: it holds `message` in its place
   * at once, and the draft is left as it is. The edit is sent again until the
   * backend answers it. Answers how the write ended; `gone` when the message
   * was sent or deleted meanwhile, which drops the edit, as no other refusal
   * does.
   */
  function rewrite(
    conversation: Conversation,
    id: string,
    message: Message,
  ): Promise<'landed' | 'waiting' | 'refused' | 'gone'> {
    const change = conversation.record.change(
      (value) => ({
        ...value,
        saved: value.saved.map((each) => each.id === id ? { ...listedAs(id, message), savedAt: each.savedAt } : each),
      }),
      {
        parts: ['saved'],
        meta: { kind: 'write', carries: null },
      },
    )
    let gone = false
    return write(conversation, change, failed('failures.saveMessage', t('failures.saveMessage')), async (signal) => {
      try {
        const answer = await putSaved(conversation.id, id, { text: message.text, files: message.files.map(namedFile) }, signal)
        return { revisions: answer.revisions }
      } catch (error) {
        gone = error instanceof ApiError && error.code === 'saved_not_found'
        throw error
      }
    }, undefined, true).then((ended) => ended === 'refused' && gone ? 'gone' : ended)
  }

  /**
   * Delete on the saved message `id`: it leaves the list at once. Answers how
   * the write ended and the message as the backend removed it, whole, which
   * Undo saves again; null when the write was refused or the message was gone.
   */
  function remove(conversation: Conversation, id: string): Promise<{ ended: 'landed' | 'waiting' | 'refused'; message: Message | null }> {
    const change = conversation.record.change((value) => withoutSaved(value, id), {
      parts: ['saved'],
      meta: { kind: 'delete', carries: null },
    })
    let removed: Message | null = null
    return write(conversation, change, failed('failures.deleteMessage', t('failures.deleteMessage')), async (signal) => {
      const answer = await deleteSaved(conversation.id, id, signal)
      removed = answer.message ? { text: answer.message.text, files: answer.message.files } : null
      return { revisions: answer.revisions }
    }).then((ended) => ({ ended, message: removed }))
  }

  /**
   * Saves `writing` for later under `id`, leaving the draft as it is: Undo
   * of a deletion, under a new id. It stands at the top of the list at once;
   * a refusal puts it back in the composer.
   */
  function resave(conversation: Conversation, id: string, writing: Message): Promise<'landed' | 'waiting' | 'refused'> {
    const message = listedAs(id, writing)
    const change = conversation.record.change((value) => ({ ...value, saved: withSaved(value.saved, message) }), {
      parts: ['saved'],
      meta: { kind: 'resave', carries: { ...writing, comments: [] } },
    })
    return write(conversation, change, failed('failures.saveMessage', t('failures.saveMessage')), async (signal) => {
      const answer = await postSaved(conversation.id, { id, text: writing.text, files: writing.files.map(namedFile) }, signal)
      return { revisions: answer.revisions }
    }, undefined, true)
  }

  /** The saved message `id` leaves the list as the message `turnId` goes, until the send ends. */
  function hideSent(conversation: Conversation, id: string, turnId: string): void {
    sends.set(turnId, conversation.record.change((value) => withoutSaved(value, id), {
      parts: ['saved'],
      meta: { kind: 'send', carries: null },
    }))
  }

  /**
   * Resolves once the conversation's writes of its record made so far have
   * ended, as a send of a saved message waits for the save that makes it;
   * the writes made after it do not wait for the send.
   */
  function settled(conversation: Conversation): Promise<void> {
    return queue.run(conversation.id, async () => {})
  }

  /** The send `turnId` ended with the message gone from the list: its change leaves once a read holds that. */
  function sent(conversation: Conversation, turnId: string): void {
    const change = sends.get(turnId)
    sends.delete(turnId)
    if (change) {
      conversation.record.land([change], {})
    }
  }

  /** The send `turnId` was taken back before the session held it: the message is listed again. */
  function unsend(conversation: Conversation, turnId: string): void {
    const change = sends.get(turnId)
    sends.delete(turnId)
    if (change) {
      conversation.record.drop([change])
    }
  }

  /**
   * Takes a draft read or one the socket brought; one no newer than the
   * draft held, such as this page's own save, whose answer's revision the
   * page holds already, changes nothing. A change restored from this web
   * browser holds only while the backend's draft is the one it was written
   * over: at another revision the backend accepted it, or a later write
   * replaced it, and the backend's draft is shown. One the backend holds
   * already only takes its revision.
   */
  function receiveDraft(conversation: Conversation, draft: ConversationDraft): void {
    const record = conversation.record
    record.receive('draft', (value) => ({ ...value, draft }), { revision: draft.revision })
    const restoredOver = restored.get(conversation.id)
    restored.delete(conversation.id)
    const unsent = typing.get(conversation.id)
    if (unsent && restoredOver !== undefined && restoredOver !== OVER_ANY && restoredOver !== draft.revision) {
      record.leave([unsent])
      typing.delete(conversation.id)
      clearTimer(conversation.id)
    } else if (unsent && record.pending('draft').length === 1 && sameWriting(unsent.meta.carries!, draft)) {
      record.leave([unsent])
      typing.delete(conversation.id)
      clearTimer(conversation.id)
    }
    follow(conversation)
    schedule(conversation, DRAFT_SAVE_DELAY_MS)
  }

  /**
   * Takes the saved messages as the conversation's metadata lists them; one
   * no newer than the list held changes nothing.
   */
  function receiveSaved(conversation: Conversation, list: SavedListing): void {
    conversation.record.receive('saved', (value) => ({ ...value, saved: list.messages }), { revision: list.revision })
  }

  /**
   * Saves every draft whose save waits for typing to pause: when the page is
   * hidden, or, with `keepalive`, when it closes. A save on close is sent at
   * once and never after another write of the conversation still in flight,
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
      } else if (!conversation.record.pending().some((change) => change.status === 'sent')) {
        void sendTyping(conversation, true)
      }
    }
  }

  function stop(): void {
    lifetime.abort()
    lifetime = new AbortController()
    for (const id of [...timers.keys()]) {
      clearTimer(id)
    }
    queue = new SerialQueue()
    typing.clear()
    restored.clear()
    sends.clear()
    seen.clear()
  }

  return {
    changed,
    save,
    follow,
    unsaved,
    over,
    restore,
    receiveDraft,
    receiveSaved,
    saveForLater,
    rewrite,
    remove,
    resave,
    hideSent,
    settled,
    sent,
    unsend,
    flush,
    stop,
  }
}
