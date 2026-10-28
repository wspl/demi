import { z } from 'zod'
import { messageCommentSchema, schemaRefusal, workflowChoiceSchema } from '@demicodes/protocol'
import { conversationSummarySchema, draftFileSchema } from '../api/generated/web-api'
import { messageEditStateSchema } from '@demicodes/web-ui/agent/message-editing'
import { modelSettingsSchema } from '@demicodes/web-ui/agent/model-selection'
import { copiedStoredFileSchema } from '@demicodes/web-ui/agent/message-input/copied-message'
import { IndexedDatabase } from '@demicodes/web-ui/infra/indexed-db'
import { pageLog } from '@demicodes/web-ui/infra/log'
import { holds } from '../state/account-owner'

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
   * with the picture its copy carried for when the paste cannot use it.
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

/**
 * `over` of a composer written over whatever the backend holds: it is kept
 * whatever revision the backend's draft reads at.
 */
export const OVER_ANY = -1

export const draftSchema = z.object({
  messageEdit: messageEditStateSchema.nullable().optional(),
  /** The messages sent and not yet confirmed, in the order they were sent. */
  pendingSends: z.array(
    z.object({
      id: z.uuid(),
      text: z.string(),
      fileIds: z.array(z.string()),
      /** The comments it carries. */
      comments: z.array(messageCommentSchema),
      error: z.string().nullable(),
      /** The request ID of the refusal that left it undelivered, shown beside `error`; absent with none. */
      requestId: z.string().optional(),
      /** The message saved for later it was sent from, which a retry sends again from the list as the same turn. */
      saved: z.string().optional(),
      /** An edit changed the saved message before it was sent: a retry sends the message as edited. */
      edited: z.literal(true).optional(),
      /** The ids of the messages sent after it, in the order they were sent, before the first of which that shows it stands while not delivered. */
      followers: z.array(z.string()).default([]),
    }),
  ),
  /**
   * The saves of messages for later and of their edits the backend has not
   * taken, each the message's id with its text and files, which go again
   * after a reload.
   */
  savedWrites: z
    .array(z.object({
      kind: z.enum(['save', 'edit']).default('edit'),
      id: z.string(),
      text: z.string(),
      files: z.array(draftFileSchema),
    }))
    .default([]),
  /**
   * The revision of the backend's draft the composer's text was written over,
   * while the backend has not accepted it, kept here and never sent; null when
   * the composer holds what the backend has, or for a conversation the backend
   * does not have yet. A page that finds the backend's draft at another
   * revision, accepted or replaced since, shows the backend's draft instead.
   */
  over: z.number().int().min(OVER_ANY).nullable(),
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
      }).extend({
        /** The workspace it was started in; absent in a draft a page kept before workspaces, which the page's own then takes. */
        workspaceId: z.string().optional(),
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
/** A Save for Later or an edit of a saved message the backend has not taken. */
export type SavedWrite = SavedDraft['savedWrites'][number]
export type SavedFile = z.infer<typeof fileSchema>

/**
 * The version of the page's draft database: raised with each change of a
 * record's shape, whose upgrade turns the previous release's records into
 * the new one.
 */
const DRAFTS_VERSION = 6

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
 * A record as version 4 stored it, whose edit in progress, always of a sent
 * message, named no kind, before a saved message was edited too.
 */
const draftV4Schema = z.looseObject({
  messageEdit: z.looseObject({}).nullable().optional(),
})

/** A record as version 5 stored it, before `over` and with its messages not saved. */
const draftV5Schema = z.looseObject({
  base: z.number().int().nonnegative().nullable().optional(),
  notSaved: z.array(z.looseObject({
    text: z.string(),
    files: z.array(draftFileSchema),
  })).optional(),
  text: z.string().optional(),
  files: z.array(z.looseObject({})).optional(),
  local: z.unknown().optional(),
})

/** A message's file as the record stores a composer's. */
function storedFile(file: z.infer<typeof draftFileSchema>): z.infer<typeof fileSchema> | z.infer<typeof remoteSchema> {
  const id = crypto.randomUUID()
  if (file.type === 'upload') {
    return {
      kind: 'file',
      id,
      name: file.fileName,
      file: null,
      upload: { id: file.ref, mediaType: file.mediaType, sha256: file.sha256 },
      ...(file.snippet ? { snippet: file.snippet } : {}),
    }
  }
  return {
    kind: 'reference',
    id,
    name: file.path.split('/').pop() || file.path,
    host: file.deviceId,
    path: file.path,
    deviceId: file.deviceId,
  }
}

/**
 * The record `value` of version 5 as version 6: `base` is `over`, and each
 * message not saved is the composer's, before its text, with its files before
 * the composer's, in the order of their marks. A composer of a conversation
 * the backend has is then one the backend does not have, written over
 * whatever it holds.
 */
function draftV5ToV6(value: unknown): unknown {
  const old = draftV5Schema.safeParse(value)
  if (!old.success) {
    return value
  }
  const { base, notSaved, ...rest } = old.data
  const brought = (notSaved ?? []).filter((message) => message.text.trim() || message.files.length)
  if (brought.length === 0) {
    return { ...rest, over: base ?? null }
  }
  const text = [...brought.map((message) => message.text), rest.text ?? ''].filter((part) => part !== '').join('\n\n')
  const files = [...brought.flatMap((message) => message.files.map(storedFile)), ...(rest.files ?? [])]
  return { ...rest, text, files, over: rest.local ? null : OVER_ANY }
}

/**
 * Upgrades the draft database from `oldVersion`:
 *
 * - A new one gets its store.
 * - One of 0.1.21 (version 1) turns each record's `pendingSend` into the list
 *   `pendingSends`.
 * - One of version 2 gives each record and each message on its way no comments,
 *   which a draft held none of then.
 * - One of version 3 drops an edit in progress whose message names a file by
 *   its path on the Host, which the backend has since numbered, so the rest of
 *   the draft stays and the user opens the edit again.
 * - One of version 4 marks an edit in progress as a sent message's.
 * - One of version 5 renames each record's `base` to `over`, and puts each
 *   message of its `notSaved`, which the page no longer keeps, back in the
 *   composer, before the text it holds, as a refused Save for Later's goes
 *   back.
 *
 * A record of another shape stays as it was, and its read drops it.
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
    if (oldVersion < 5) {
      const edited = draftV4Schema.safeParse(value)
      if (edited.success && edited.data.messageEdit) {
        value = { ...edited.data, messageEdit: { kind: 'message', ...edited.data.messageEdit } }
      }
    }
    if (oldVersion < 6) {
      value = draftV5ToV6(value)
    }
    if (value !== cursor.value) {
      cursor.update(value)
    }
    cursor.continue()
  }
}

/** The record `value` as a draft; none when it does not decode. */
function decodeDraft(value: unknown): SavedDraft | null {
  const draft = draftSchema.safeParse(value)
  return draft.success ? draft.data : null
}

/** Calls `listener` when the draft database was deleted, as by another page; answers the call that stops it. */
export function onDraftStorageDeleted(listener: () => void): () => void {
  return drafts.onDeleted(listener)
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

/**
 * Deletes the records under `keys` that still do not decode. The local data is
 * disposable, so a record of a shape this page cannot read, as a newer build
 * wrote, is dropped and the page goes on without it. A deletion that fails
 * leaves the record to the next read.
 */
function dropUnreadable(keys: readonly IDBValidKey[]): Promise<void> {
  return drafts
    .across(['drafts'], 'readwrite', (transaction) => {
      const store = transaction.objectStore('drafts')
      for (const key of keys) {
        const request = store.get(key)
        request.onsuccess = () => {
          const draft = draftSchema.safeParse(request.result)
          if (request.result !== undefined && !draft.success) {
            pageLog.warn('storage.dropped', { database: 'demi-drafts', record: String(key), reason: schemaRefusal(draft.error) })
            store.delete(key)
          }
        }
      }
      return () => undefined
    })
    .catch(() => {})
}

/** The conversation's draft; none when it has none or its record does not decode, which is then dropped. */
export async function readDraft(
  userId: string,
  conversationId: string,
): Promise<SavedDraft | null> {
  const key = [userId, conversationId]
  const value = await draftStorage('readonly', (store) => store.get(key))
  if (value === undefined) {
    return null
  }
  const draft = decodeDraft(value)
  if (draft === null) {
    await dropUnreadable([key])
  }
  return draft
}

export async function writeDraft(
  userId: string,
  conversationId: string,
  draft: SavedDraft,
): Promise<void> {
  // A page whose account no longer holds the browser's data writes none of it.
  if (!holds(userId)) {
    return
  }
  await draftStorage('readwrite', (store) =>
    store.put(draft, [userId, conversationId]),
  )
}

/** The drafts of the user's new conversations; a record that does not decode is dropped, and every other is read. */
export async function readLocalDrafts(userId: string): Promise<SavedDraft[]> {
  const range = IDBKeyRange.bound([userId], [userId, []])
  const { values, keys } = await drafts.across(['drafts'], 'readonly', (transaction) => {
    const store = transaction.objectStore('drafts')
    const found = store.getAll(range)
    const named = store.getAllKeys(range)
    return () => ({ values: found.result as unknown[], keys: named.result })
  })
  const decoded = values.map(decodeDraft)
  await dropUnreadable(keys.filter((_, index) => decoded[index] === null))
  return decoded.filter((draft): draft is SavedDraft => draft !== null && draft.local !== null)
}

/**
 * Deletes every draft but those of the account `keep`, every one for none:
 * the browser forgets an account's drafts with the account.
 */
export async function deleteDraftsBut(keep: string | null): Promise<void> {
  await drafts.across(['drafts'], 'readwrite', (transaction) => {
    const cursor = transaction.objectStore('drafts').openCursor()
    cursor.onsuccess = () => {
      const at = cursor.result
      if (!at) {
        return
      }
      if (keep === null || !Array.isArray(at.key) || at.key[0] !== keep) {
        at.delete()
      }
      at.continue()
    }
    return () => undefined
  })
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
 * What a conversation's page held as it closed: the composer's text the backend
 * does not have, with the revision it was written over and the files of its marks,
 * and the messages on their way, with what their files are without their bytes,
 * which the IndexedDB record carries. An IndexedDB write still under way when
 * the page goes can be lost, such as a send's, whose files make it long, so the
 * page writes this where the web browser writes at once, and the next page of
 * the conversation takes it over the record.
 */
const lastWordsSchema = z.object({
  composer: z
    .object({
      over: z.number().int().min(OVER_ANY).nullable().default(OVER_ANY),
      text: z.string(),
      attachmentIds: z.array(z.string()),
    })
    .nullable(),
  pendingSends: draftSchema.shape.pendingSends,
  savedWrites: draftSchema.shape.savedWrites,
  files: draftSchema.shape.files,
})
export type LastWords = z.infer<typeof lastWordsSchema>

function lastWordsKey(userId: string, conversationId: string): string {
  return `demi.draft.${userId}.${conversationId}`
}

export function writeLastWords(userId: string, conversationId: string, words: LastWords): void {
  if (!holds(userId)) {
    return
  }
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
