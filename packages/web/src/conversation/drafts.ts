import { z } from 'zod'
import { messageCommentSchema, schemaRefusal } from '@demicodes/protocol'
import { conversationSummarySchema, draftFileSchema, workflowChoiceSchema } from '../api/generated/web-api'
import { messageEditStateSchema } from '@demicodes/web-ui/agent/message-editing'
import { modelSettingsSchema } from '@demicodes/web-ui/agent/model-selection'
import { copiedStoredFileSchema } from '@demicodes/web-ui/agent/message-input/copied-message'
import { IndexedDatabase } from '../state/indexed-db'

/**
 * The upload the backend took: the attachment id a message names the file
 * by, and where its bytes are, which a capsule's picture loads from.
 */
const uploadSchema = z.object({ id: z.string(), mediaType: z.string(), sha256: z.string() })
const fileSchema = z.object({
  kind: z.literal('file'),
  id: z.string(),
  name: z.string(),
  /** The bytes this page added; none for a file another page uploaded. */
  file: z.instanceof(File).nullable(),
  upload: uploadSchema.nullable(),
  /**
   * The stored file a pasted message names, while it is not attached yet,
   * with the picture its copy carried for when the paste cannot use it
   * (`composer.md` § Writing a message).
   */
  claim: z.object({ file: copiedStoredFileSchema, picture: z.instanceof(File).nullable() }).optional(),
  /** A text file's opening, as the backend answered its upload. */
  snippet: z.string().optional(),
})
const remoteSchema = z.object({
  kind: z.literal('reference'),
  id: z.string(),
  name: z.string(),
  host: z.string(),
  path: z.string(),
  deviceId: z.string(),
})
export const draftSchema = z.object({
  messageEdit: messageEditStateSchema.nullable().optional(),
  /** The messages sent and not yet confirmed, in the order they were sent. */
  pendingSends: z.array(
    z.object({
      id: z.uuid(),
      text: z.string(),
      fileIds: z.array(z.string()),
      /** The comments it carries (`comments-and-ask.md` § Sending them). */
      comments: z.array(messageCommentSchema),
      error: z.string().nullable(),
      /** The request ID of the refusal that left it undelivered, shown beside `error`; absent with none. */
      requestId: z.string().optional(),
      /** The message saved for later it was sent from, which a retry sends again from the list as the same turn. */
      saved: z.string().optional(),
    }),
  ),
  /** The messages the backend did not save, which the list shows as Not Saved until their user retries or deletes them. */
  notSaved: z
    .array(z.object({ id: z.string(), text: z.string(), files: z.array(draftFileSchema), reason: z.string() }))
    .default([]),
  /**
   * The revision of the backend's draft the composer's text was built on,
   * while the backend has not confirmed a change of it; null when the
   * composer holds what the backend has, or for a conversation the backend
   * does not have yet.
   */
  base: z.number().int().nonnegative().nullable(),
  local: z
    .object({
      phase: z.enum(['draft', 'pending']),
      conversation: conversationSummarySchema.pick({
        id: true,
        title: true,
        pinned: true,
        archived: true,
        target: true,
        createdAt: true,
        updatedAt: true,
      }),
    })
    .nullable(),
  /** The composer's text, when the backend does not have it. */
  text: z.string(),
  /** The comments waiting with that text, when the backend does not have them. */
  comments: z.array(messageCommentSchema),
  /** The model settings of a conversation without a record; null for one with a record, which holds them. */
  model: modelSettingsSchema.nullable(),
  /**
   * The Workflow menu choice of a conversation without a record; absent
   * for Default, and for one with a record, which holds it.
   */
  workflow: workflowChoiceSchema.optional(),
  /** The files of that text, then those of a send not yet accepted. */
  files: z.array(z.union([fileSchema, remoteSchema])),
  scroll: z
    .object({
      anchor: z.object({
        blockId: z.string(),
        anchorIndex: z.number(),
        offsetPx: z.number(),
        scrollTop: z.number(),
      }),
      heightCache: z.map(z.string(), z.number()),
    })
    .nullable(),
})
export type SavedDraft = z.infer<typeof draftSchema>
export type SavedFile = z.infer<typeof fileSchema>

/**
 * The version of the page's draft database: raised with each change of a
 * record's shape, whose upgrade turns the previous release's records into
 * the new one (`web-application.md` § Persistence and adapters).
 */
const DRAFTS_VERSION = 4

/** A message on its way as release 0.1.21 stored it: one per conversation, or none. */
const pendingSendV1Schema = z.looseObject({
  pendingSend: draftSchema.shape.pendingSends.element.omit({ comments: true }).nullable(),
})

/** A record as version 2 stored it, before a draft held comments. */
const draftV2Schema = z.looseObject({
  pendingSends: z.array(z.looseObject({})),
})

/**
 * A record as version 3 stored it, whose edit in progress may hold an
 * attachment record that named its file by its path on the Host, before an
 * attachment had a number.
 */
const draftV3Schema = z.looseObject({
  messageEdit: z.looseObject({ request: z.looseObject({ content: z.array(z.looseObject({})) }) }).nullable().optional(),
})

/**
 * Upgrades the draft database from `oldVersion`: a new one gets its store;
 * one of 0.1.21 (version 1) turns each record's `pendingSend` into the list
 * `pendingSends`; one of version 2 gives each record and each message on
 * its way no comments, which a draft held none of then; one of version 3
 * drops an edit in progress whose message names a file by its path on the
 * Host, which the backend has since numbered, so the rest of the draft
 * stays and the user opens the edit again. A record of another shape stays
 * as it was, and its read reports it.
 */
function upgradeDrafts(request: IDBOpenDBRequest, oldVersion: number): void {
  if (oldVersion < 1) {
    request.result.createObjectStore('drafts')
    return
  }
  const cursors = request.transaction!.objectStore('drafts').openCursor()
  cursors.onsuccess = () => {
    const cursor = cursors.result
    if (!cursor) {
      return
    }
    let value: unknown = cursor.value
    if (oldVersion < 2) {
      const old = pendingSendV1Schema.safeParse(value)
      if (old.success) {
        const { pendingSend, ...rest } = old.data
        value = { ...rest, pendingSends: pendingSend ? [pendingSend] : [] }
      }
    }
    const before = oldVersion < 3 ? draftV2Schema.safeParse(value) : null
    if (before?.success) {
      value = {
        ...before.data,
        comments: [],
        pendingSends: before.data.pendingSends.map((pending) => ({ ...pending, comments: [] })),
      }
    }
    if (oldVersion < 4) {
      const edited = draftV3Schema.safeParse(value)
      const byPath = edited.success && edited.data.messageEdit?.request.content.some(
        (part) => part.type === 'attachment' && 'path' in part,
      )
      if (edited.success && byPath) {
        value = { ...edited.data, messageEdit: null }
      }
    }
    if (value !== cursor.value) {
      cursor.update(value)
    }
    cursor.continue()
  }
}

/** A record a read could not decode: it is set aside, and every other is read. */
export class UnreadableDraftError extends Error {
  constructor(readonly detail: string) {
    super('A draft stored in this web browser could not be read.')
  }
}

/** The record `value` as a draft, or why it is unreadable. */
function decodeDraft(value: unknown): SavedDraft | UnreadableDraftError {
  const draft = draftSchema.safeParse(value)
  return draft.success ? draft.data : new UnreadableDraftError(schemaRefusal(draft.error))
}

const drafts = new IndexedDatabase('demi-drafts', DRAFTS_VERSION, upgradeDrafts)

/** Closes the draft database's connection; writes already issued finish, and the next use opens it again. */
export function closeDraftStorage(): void {
  drafts.close()
}

function draftStorage<T>(
  mode: IDBTransactionMode,
  operation: (store: IDBObjectStore) => IDBRequest<T>,
): Promise<T> {
  return drafts.transaction('drafts', mode, operation)
}

/** The conversation's draft; rejects with `UnreadableDraftError` when its record does not decode. */
export async function readDraft(
  userId: string,
  conversationId: string,
): Promise<SavedDraft | null> {
  const value = await draftStorage('readonly', (store) =>
    store.get([userId, conversationId]),
  )
  if (value === undefined) {
    return null
  }
  const draft = decodeDraft(value)
  if (draft instanceof UnreadableDraftError) {
    throw draft
  }
  return draft
}

export async function writeDraft(
  userId: string,
  conversationId: string,
  draft: SavedDraft,
): Promise<void> {
  await draftStorage('readwrite', (store) =>
    store.put(draft, [userId, conversationId]),
  )
}

/**
 * The drafts of the user's new conversations, and the records that did not
 * decode, which are set aside: one never keeps the others from being read.
 */
export async function readLocalDrafts(userId: string): Promise<{ drafts: SavedDraft[]; unreadable: UnreadableDraftError[] }> {
  const values = await draftStorage('readonly', (store) =>
    store.getAll(IDBKeyRange.bound([userId], [userId, []])),
  )
  const decoded = values.map(decodeDraft)
  return {
    drafts: decoded.filter((draft): draft is SavedDraft => !(draft instanceof UnreadableDraftError) && draft.local !== null),
    unreadable: decoded.filter((draft) => draft instanceof UnreadableDraftError),
  }
}

export async function deleteDraft(
  userId: string,
  conversationId: string,
): Promise<void> {
  await draftStorage('readwrite', (store) =>
    store.delete([userId, conversationId]),
  )
}

/**
 * What a conversation's page held as it closed (`web-application.md`
 * § Drafts): the composer's text the backend does not have, with the
 * revision it was built on and the files of its marks, and the messages on
 * their way, with what their files are without their bytes, which the
 * IndexedDB record carries. An IndexedDB write still under way when the page
 * goes can be lost, such as a send's, whose files make it long, so the page
 * writes this where the web browser writes at once, and the next page of the
 * conversation takes it over the record.
 */
const lastWordsSchema = z.object({
  composer: z
    .object({
      base: z.number().int().nonnegative().nullable(),
      text: z.string(),
      attachmentIds: z.array(z.string()),
    })
    .nullable(),
  pendingSends: draftSchema.shape.pendingSends,
  files: draftSchema.shape.files,
})
export type LastWords = z.infer<typeof lastWordsSchema>

function lastWordsKey(userId: string, conversationId: string): string {
  return `demi.draft.${userId}.${conversationId}`
}

export function writeLastWords(userId: string, conversationId: string, words: LastWords): void {
  try {
    localStorage.setItem(lastWordsKey(userId, conversationId), JSON.stringify(words))
  } catch {
    // Storage the web browser refuses, as in a private window, leaves the
    // IndexedDB record, a moment older, to the next page.
  }
}

/** The text a page of the conversation left as it closed, once; none when it left none. */
export function takeLastWords(userId: string, conversationId: string): LastWords | null {
  const key = lastWordsKey(userId, conversationId)
  try {
    const kept = localStorage.getItem(key)
    localStorage.removeItem(key)
    return kept === null ? null : lastWordsSchema.parse(JSON.parse(kept))
  } catch {
    // Unreadable storage, or an entry that is not ours: the IndexedDB record
    // stands, as it would had the page left nothing here.
    return null
  }
}
