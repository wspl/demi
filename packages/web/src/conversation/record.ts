import { errorMessage } from '@demicodes/web-ui/infra/errors'
import { markRaw, type Raw } from 'vue'
import type { ClientContent, MessageComment } from '@demicodes/protocol'
import { SerialQueue } from '@demicodes/utils'
import type { HeadlineText } from '@demicodes/web-ui/ui/ui-text'
import { Slot, type SlotChange } from '@demicodes/web-ui/infra/slot'
import { isComposerFile } from '@demicodes/web-ui/agent/message-input/attachments'
import { ATTACHMENT_MARK } from '@demicodes/web-ui/markdown/user-markdown'
import { ApiError } from '../api/client'
import { changeReplacedDraft, loadDraft, saveDraft } from '../api/drafts'
import { deleteSaved, editSaved, loadSaved, postSaved, saveFromDraft } from '../api/saved'
import type { ConversationDraft, DraftFile, ReplacedAction, Revisions, SavedMessage } from '../api/generated/web-api'
import type { Conversation, ProductAttachment } from '../state/types'
import { t } from './locales/text'

// A conversation's draft and its messages saved for later, which the backend
// changes together, as the one slot the page holds of them
// (`web-application.md` § Responding to the user, § Drafts): every write of
// either is a change of the slot, sent in the order made after the writes
// before it, and its answer names the revisions it produced, so the change
// leaves once a read of them is held. The composer shows the slot's draft:
// what its user types is a change of it, sent half a second after typing
// pauses; a newer draft read shows in it unless its user has a change the
// backend has not confirmed. Text the user wrote is never only in a change:
// a refused one goes back to the composer or stands in the list as Not Saved.

/** How long typing pauses before the page saves the draft. */
export const DRAFT_SAVE_DELAY_MS = 500

/** How long a save that failed waits before it is tried again. */
export const DRAFT_RETRY_MS = 3_000

/** The draft and the saved messages as the page holds them. */
export interface ConversationRecord {
  draft: ConversationDraft
  saved: SavedMessage[]
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

/** A message the backend did not save, which the list shows as Not Saved until its user retries or deletes it. */
export interface NotSaved extends Message {
  id: string
  /** Why it was not saved, as the backend or the page said it. */
  reason: string
}

/** What the composer's text was built on: a revision of the draft read, or a change of the page's below it. */
type Basis = number | RecordChange

/** What the page keeps with a change of the record. */
export interface RecordMeta {
  kind: 'typing' | 'save' | 'edit' | 'delete' | 'resave' | 'send'
  /** For a change that sets the draft, what the text below it was built on. */
  base: Basis | null
  /** The revision of the draft its answer produced. */
  answered: number | null
  /**
   * The user's writing the change carries, which a refusal must not lose,
   * and where it goes back to: the composer, or the list under its id.
   */
  carries: (Writing & { id: string; into: 'draft' | 'list' }) | null
}

export type RecordChange = SlotChange<ConversationRecord, RecordMeta>
export type RecordSlot = Slot<ConversationRecord, RecordMeta>

/** A conversation's record as the page holds it before reading anything. */
export function newRecord(): Raw<RecordSlot> {
  const slot = new Slot<ConversationRecord, RecordMeta>()
  slot.replace({ draft: { revision: 0, text: '', files: [], comments: [], replaced: null }, saved: [] })
  return markRaw(slot)
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
function draftFile(file: ProductAttachment): DraftFile | null {
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
function withSaved(list: SavedMessage[], message: SavedMessage): SavedMessage[] {
  return list.some((each) => each.id === message.id) ? list : [message, ...list]
}

/** `record` without the saved message `id`; the same record when it has none. */
function withoutSaved(record: ConversationRecord, id: string): ConversationRecord {
  return record.saved.some((message) => message.id === id)
    ? { ...record, saved: record.saved.filter((message) => message.id !== id) }
    : record
}

/** A refusal the backend answered, which sending again would not change. */
function isRefusal(error: unknown): boolean {
  return error instanceof ApiError && error.status >= 400 && error.status < 500
}

/** The words a refusal stands in the list with. */
function reasonOf(error: unknown): string {
  return errorMessage(error)
}

export function createRecordSync(options: {
  /** Shows `writing` in the conversation's composer, with its files. */
  show: (conversation: Conversation, writing: Writing) => void
  /** Makes the conversation's backend record, when it has none yet. */
  persist: (conversation: Conversation) => Promise<void>
  report: (title: HeadlineText, error: unknown) => void
}) {
  let lifetime = new AbortController()
  const timers = new Map<string, ReturnType<typeof setTimeout>>()
  /** Each conversation's writes of its record, in order. */
  let queue = new SerialQueue()
  /** Each conversation's change of its draft that its user types and the page has not sent yet. */
  const typing = new Map<string, RecordChange>()
  /** What each conversation's composer text was built on, while the composer shows the slot's draft. */
  const builtOn = new Map<string, Basis>()
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

  /** The revision of the draft that `basis` stands for once its writes are answered. */
  function revisionOf(basis: Basis | null | undefined): number {
    if (basis === null || basis === undefined) {
      return 0
    }
    if (typeof basis === 'number') {
      return Math.max(0, basis)
    }
    return basis.meta.answered ?? revisionOf(basis.meta.base)
  }

  /** What the slot's draft is built on now: the last change of it, or the revision read. */
  function basis(conversation: Conversation): Basis {
    return conversation.record.pending('draft').at(-1) ?? conversation.record.revision('draft')
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
    builtOn.set(conversation.id, basis(conversation))
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
      conversation.record.pending('draft').some((change) => change.status !== 'landed') ||
      !sameWriting(writing, shownDraft(conversation))
  }

  /** The revision the composer's text was built on, as the web browser keeps it with an unconfirmed change. */
  function base(conversation: Conversation): number {
    return revisionOf(builtOn.get(conversation.id) ?? conversation.record.revision('draft'))
  }

  /** A composer this web browser kept builds on `revision`, the one it was built on then. */
  function restore(conversation: Conversation, revision: number | null): void {
    if (revision !== null) {
      builtOn.set(conversation.id, revision)
    }
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
    if ((seen.get(conversation.id) ?? held({ text: '', files: [], comments: [] })) === held(writing)) {
      return
    }
    seen.set(conversation.id, held(writing))
    const unsent = typing.get(conversation.id)
    const below = unsent ? record.below(unsent)! : record.shown.value!
    if (sameWriting(writing, below.draft)) {
      if (unsent) {
        record.leave([unsent])
        typing.delete(conversation.id)
        clearTimer(conversation.id)
      }
      return
    }
    const carries = { id: crypto.randomUUID(), ...writing, into: 'draft' as const }
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
        meta: { kind: 'typing', base: builtOn.get(conversation.id) ?? record.revision('draft'), answered: null, carries },
      })
      typing.set(conversation.id, change)
      builtOn.set(conversation.id, change)
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
   * once the backend has the message. `detached`, a change an edit sends
   * first, goes whatever waits, since the edit moves what it holds.
   */
  async function sendTyping(conversation: Conversation, keepalive = false, detached?: RecordChange): Promise<void> {
    const change = detached ?? typing.get(conversation.id)
    if (!change || !conversation.record.pending().includes(change) || conversation.persistence !== 'synced' ||
      conversation.archived || (!detached && conversation.pendingSends.length > 0)) {
      return
    }
    if (typing.get(conversation.id) === change) {
      typing.delete(conversation.id)
      clearTimer(conversation.id)
    }
    const signal = lifetime.signal
    const writing = change.meta.carries!
    const revision = revisionOf(change.meta.base)
    change.send()
    try {
      const answer = await saveDraft(
        conversation.id,
        { base: revision, text: writing.text, files: writing.files.map(namedFile), comments: writing.comments },
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
      refused(conversation, change, isRefusal(error) ? t('failures.saveDraft') : null, error)
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
      change.meta.answered = answer.revisions.draft
      conversation.record.receive('draft', (value) => ({ ...value, draft }), { revision: answer.revisions.draft })
    }
    const left = conversation.record.land([change], { revisions: answer.revisions })
    follow(conversation)
    return left
  }

  /**
   * A write of `change` failed: the change leaves, the failure is reported
   * when `title` names it, and what the user wrote in it is kept. Writing
   * the list or the composer shows already stays there; the composer's own
   * goes back to it while no later change of the draft took its place, and
   * any other stands in the list as Not Saved, the composer not showing it a
   * second time.
   */
  function refused(conversation: Conversation, change: RecordChange, title: HeadlineText | null, error: unknown): void {
    const record = conversation.record
    const carried = change.meta.carries
    record.drop([change])
    if (title) {
      options.report(title, error)
    }
    if (carried && !isBlank(carried)) {
      const shown = record.shown.value!
      const listed = shown.saved.some((message) => sameMessage(message, carried))
      if (!listed && carried.into === 'draft') {
        if (!sameWriting(shown.draft, carried) && !typing.has(conversation.id)) {
          if (record.pending('draft').length === 0) {
            const again = record.change(
              (value) => ({ ...value, draft: { ...value.draft, text: carried.text, files: carried.files, comments: carried.comments } }),
              { parts: ['draft'], meta: { kind: 'typing', base: change.meta.base, answered: null, carries: carried } },
            )
            typing.set(conversation.id, again)
          } else {
            keepNotSaved(conversation, carried, error)
          }
        }
      } else if (!listed) {
        keepNotSaved(conversation, carried, error)
        if (sameMessage(shown.draft, carried)) {
          // It stands in the list now: the composer, which would show it
          // again, is emptied, a change of the draft as any; its comments
          // stay waiting.
          const comments = shown.draft.comments
          const empty = record.change(
            (value) => ({ ...value, draft: { ...value.draft, text: '', files: [] } }),
            {
              parts: ['draft'],
              meta: { kind: 'typing', base: basis(conversation), answered: null, carries: { id: crypto.randomUUID(), text: '', files: [], comments, into: 'draft' } },
            },
          )
          typing.set(conversation.id, empty)
          schedule(conversation, DRAFT_SAVE_DELAY_MS)
        }
      }
    }
    follow(conversation)
  }

  /** Keeps `writing` in the list as Not Saved, under its id, newest first. */
  function keepNotSaved(conversation: Conversation, writing: Message & { id: string }, error: unknown): void {
    const row: NotSaved = { id: writing.id, text: writing.text, files: writing.files, reason: reasonOf(error) }
    conversation.notSaved = [row, ...conversation.notSaved.filter((each) => each.id !== row.id)]
  }

  /**
   * Writes `change` after the conversation's writes before it: `request`
   * answers what the write produced. `sending` runs as it goes out, for a
   * change that settles what it shows then. A failure is a refusal of the
   * change (`refused`). Answers how it ended.
   */
  function write(
    conversation: Conversation,
    change: RecordChange,
    title: HeadlineText,
    request: (signal: AbortSignal) => Promise<{ draft?: ConversationDraft; revisions: Revisions }>,
    sending?: () => void,
  ): Promise<'landed' | 'waiting' | 'refused'> {
    const signal = lifetime.signal
    return queue.run(conversation.id, async () => {
      if (!conversation.record.pending().includes(change)) {
        return 'refused' as const
      }
      sending?.()
      change.send()
      try {
        const answer = await request(signal)
        signal.throwIfAborted()
        return landed(conversation, change, answer) ? 'landed' as const : 'waiting' as const
      } catch (error) {
        if (!signal.aborted) {
          refused(conversation, change, title, error)
        }
        return 'refused' as const
      }
    })
  }

  /**
   * Save for Later: `writing`, the composer's message, stands at the top of
   * the list under `id` and leaves the composer at once; the backend keeps
   * it and empties the draft with it, after the writes before it.
   */
  function saveForLater(conversation: Conversation, id: string, writing: Message): Promise<'landed' | 'waiting' | 'refused'> {
    const record = conversation.record
    // The message is the composer's: the change its user typed is in it.
    const unsent = typing.get(conversation.id)
    const built = unsent?.meta.base ?? builtOn.get(conversation.id) ?? record.revision('draft')
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
    const message: SavedMessage = { id, text: writing.text, files: writing.files, savedAt: new Date().toISOString() }
    // The draft's comments stay waiting there, as the backend keeps them.
    const change = record.change(
      (value) => ({ draft: { ...value.draft, text: '', files: [] }, saved: withSaved(value.saved, message) }),
      {
        parts: ['draft', 'saved'],
        meta: { kind: 'save', base: built, answered: null, carries: { id, text: writing.text, files: writing.files, comments: [], into: 'list' } },
      },
    )
    const kept = unsaved
      ? record.change(
        (value) => ({ ...value, draft: { ...value.draft, text: '', files: [], comments } }),
        {
          parts: ['draft'],
          meta: { kind: 'typing', base: change, answered: null, carries: { id: crypto.randomUUID(), text: '', files: [], comments, into: 'draft' } },
        },
      )
      : null
    if (kept) {
      builtOn.set(conversation.id, kept)
    }
    follow(conversation)
    const saved = write(conversation, change, t('failures.saveMessage'), async (signal) => {
      await options.persist(conversation)
      return saveFromDraft(
        conversation.id,
        { id, base: revisionOf(built), text: writing.text, files: writing.files.map(namedFile) },
        signal,
      )
    })
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

  /**
   * Edit on the saved message `message`: it goes into the composer and leaves
   * the list at once, and what the composer holds stands in the list in its
   * place, as the backend moves it, under an id the page names. What the
   * composer holds is saved first, so the backend moves that; until the edit
   * goes out it is whatever the draft below it is.
   */
  function edit(conversation: Conversation, message: SavedMessage): Promise<'landed' | 'waiting' | 'refused'> {
    const record = conversation.record
    // The change its user typed goes first, and is typed on no more.
    const unsent = typing.get(conversation.id)
    if (unsent) {
      typing.delete(conversation.id)
      clearTimer(conversation.id)
      void queue.run(conversation.id, () => sendTyping(conversation, false, unsent))
    }
    const becomes = crypto.randomUUID()
    const op = crypto.randomUUID()
    // The draft's text and files go into the list; its comments stay waiting.
    const keptOf = (draft: ConversationDraft) =>
      isBlankMessage(draft) ? null : { id: becomes, text: draft.text, files: draft.files, comments: [], into: 'list' as const }
    let kept: RecordMeta['carries'] | undefined
    const change = record.change(
      (value) => {
        const moved = kept === undefined ? keptOf(value.draft) : kept
        const rest = value.saved.filter((each) => each.id !== message.id)
        return {
          draft: { ...value.draft, text: message.text, files: message.files },
          saved: moved ? withSaved(rest, { id: moved.id, text: moved.text, files: moved.files, savedAt: new Date().toISOString() }) : rest,
        }
      },
      { parts: ['draft', 'saved'], meta: { kind: 'edit', base: basis(conversation), answered: null, carries: null } },
    )
    follow(conversation)
    return write(
      conversation,
      change,
      t('failures.editMessage'),
      (signal) => editSaved(conversation.id, message.id, { op, id: becomes }, signal),
      () => {
        // The writes before it are answered: what the backend moves is the
        // draft below it now, which the change shows from here on.
        kept = keptOf(record.below(change)!.draft)
        change.meta.carries = kept
        record.touched()
      },
    )
  }

  /** Delete on the saved message `id`: it leaves the list at once. */
  function remove(conversation: Conversation, id: string): Promise<'landed' | 'waiting' | 'refused'> {
    const change = conversation.record.change((value) => withoutSaved(value, id), {
      parts: ['saved'],
      meta: { kind: 'delete', base: null, answered: null, carries: null },
    })
    return write(conversation, change, t('failures.deleteMessage'), async (signal) => ({
      revisions: await deleteSaved(conversation.id, id, signal),
    }))
  }

  /**
   * Saves `writing` for later under `id`, leaving the draft as it is: Undo
   * of a deletion, under a new id, and Retry of a message not saved, under
   * its own. It stands at the top of the list at once.
   */
  function resave(conversation: Conversation, id: string, writing: Message): Promise<'landed' | 'waiting' | 'refused'> {
    const message: SavedMessage = { id, text: writing.text, files: writing.files, savedAt: new Date().toISOString() }
    const change = conversation.record.change((value) => ({ ...value, saved: withSaved(value.saved, message) }), {
      parts: ['saved'],
      meta: { kind: 'resave', base: null, answered: null, carries: { id, text: writing.text, files: writing.files, comments: [], into: 'list' } },
    })
    return write(conversation, change, t('failures.saveMessage'), async (signal) => {
      const answer = await postSaved(conversation.id, { id, text: writing.text, files: writing.files.map(namedFile) }, signal)
      return { revisions: answer.revisions }
    })
  }

  /** The saved message `id` leaves the list as the message `turnId` goes, until the send ends. */
  function hideSent(conversation: Conversation, id: string, turnId: string): void {
    sends.set(turnId, conversation.record.change((value) => withoutSaved(value, id), {
      parts: ['saved'],
      meta: { kind: 'send', base: null, answered: null, carries: null },
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

  /** Takes a draft read; a change restored from this web browser that the backend holds already only takes its revision. */
  function receiveDraft(conversation: Conversation, draft: ConversationDraft): void {
    const record = conversation.record
    record.receive('draft', (value) => ({ ...value, draft }), { revision: draft.revision })
    const unsent = typing.get(conversation.id)
    if (unsent && record.pending('draft').length === 1 && sameWriting(unsent.meta.carries!, draft)) {
      record.leave([unsent])
      typing.delete(conversation.id)
      clearTimer(conversation.id)
    }
    follow(conversation)
    schedule(conversation, DRAFT_SAVE_DELAY_MS)
  }

  /**
   * Reads the draft, or takes `loaded`, a read of it already under way; its
   * revision orders it against the writes' answers. A change the backend
   * has not confirmed, such as one this page restored from the web
   * browser's storage, is saved after it.
   */
  async function readDraft(conversation: Conversation, loaded?: Promise<ConversationDraft>): Promise<void> {
    const signal = lifetime.signal
    const draft = await (loaded ?? loadDraft(conversation.id, signal))
    signal.throwIfAborted()
    receiveDraft(conversation, draft)
  }

  /**
   * Reads the saved messages; a message not saved that the list holds now
   * was saved after all, and is listed as saved.
   */
  async function readSaved(conversation: Conversation): Promise<void> {
    const since = conversation.record.now()
    const list = await loadSaved(conversation.id, lifetime.signal)
    conversation.record.receive('saved', (value) => ({ ...value, saved: list.messages }), { revision: list.revision, since })
    if (conversation.notSaved.some((row) => list.messages.some((message) => message.id === row.id))) {
      conversation.notSaved = conversation.notSaved.filter((row) => !list.messages.some((message) => message.id === row.id))
    }
  }

  /**
   * Restores or dismisses the replaced version the composer offers, after
   * saving what the composer holds. The answer shows in the composer; a
   * replaced version that changed meanwhile is read again, so the composer
   * offers the one there is.
   */
  function act(conversation: Conversation, action: ReplacedAction): Promise<void> {
    const signal = lifetime.signal
    clearTimer(conversation.id)
    return queue.run(conversation.id, async () => {
      await sendTyping(conversation)
      const replaced = conversation.record.read.value?.draft.replaced
      if (!replaced) {
        return
      }
      try {
        const answer = await changeReplacedDraft(conversation.id, { action, revision: replaced.revision }, signal)
        signal.throwIfAborted()
        conversation.record.receive('draft', (value) => ({ ...value, draft: answer.draft }), { revision: answer.revisions.draft })
        follow(conversation)
      } catch (error) {
        if (!(error instanceof ApiError && error.code === 'draft_changed')) {
          throw error
        }
        const draft = await loadDraft(conversation.id, signal)
        signal.throwIfAborted()
        receiveDraft(conversation, draft)
      }
    })
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
    builtOn.clear()
    sends.clear()
    seen.clear()
  }

  return {
    changed,
    save,
    follow,
    unsaved,
    base,
    restore,
    readDraft,
    readSaved,
    act,
    saveForLater,
    edit,
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
