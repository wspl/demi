import { z } from 'zod'
import { attachedHostSchema, conversationSummarySchema } from '../api/generated/web-api'
import { messageEditStateSchema } from '@demicodes/web-ui/agent/message-editing'
import { modelSettingsSchema } from '@demicodes/web-ui/agent/model-selection'

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
  pendingSend: z
    .object({
      id: z.uuid(),
      text: z.string(),
      fileIds: z.array(z.string()),
      error: z.string().nullable(),
    })
    .nullable(),
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
      hosts: z.array(attachedHostSchema.pick({ deviceId: true, name: true, cwd: true })),
    })
    .nullable(),
  /** The composer's text, when the backend does not have it. */
  text: z.string(),
  /** The model settings of a conversation without a record; null for one with a record, which holds them. */
  model: modelSettingsSchema.nullable(),
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

let database: IDBDatabase | null = null
let opening: Promise<IDBDatabase> | null = null
let generation = 0

function openStorage(): Promise<IDBDatabase> {
  if (opening) {
    return opening
  }
  const current = generation
  opening = new Promise<IDBDatabase>((resolve, reject) => {
    let failed = false
    const request = indexedDB.open('demi-drafts', 1)
    request.onupgradeneeded = () => request.result.createObjectStore('drafts')
    request.onsuccess = () => {
      const db = request.result
      if (failed || current !== generation) {
        db.close()
        reject(new DOMException('Draft storage closed.', 'AbortError'))
        return
      }
      database = db
      db.onversionchange = closeDraftStorage
      resolve(db)
    }
    request.onerror = () => reject(request.error)
    request.onblocked = () => {
      failed = true
      reject(new Error('Draft storage is blocked.'))
    }
  }).finally(() => {
    if (current === generation) {
      opening = null
    }
  })
  return opening
}

/** Closing a handle lets already-issued transactions finish. */
export function closeDraftStorage(): void {
  generation += 1
  database?.close()
  database = null
  opening = null
}

/** Keep the handle open so pagehide can issue writes before document teardown. */
export async function draftStorage<T>(
  mode: IDBTransactionMode,
  operation: (store: IDBObjectStore) => IDBRequest<T>,
): Promise<T> {
  const db = database ?? (await openStorage())
  return new Promise<T>((resolve, reject) => {
    const transaction = db.transaction('drafts', mode)
    const request = operation(transaction.objectStore('drafts'))
    transaction.oncomplete = () => resolve(request.result)
    transaction.onerror = () => reject(transaction.error)
    transaction.onabort = () =>
      reject(transaction.error ?? new Error('Draft write aborted.'))
  })
}

export async function readDraft(
  userId: string,
  conversationId: string,
): Promise<SavedDraft | null> {
  const value = await draftStorage('readonly', (store) =>
    store.get([userId, conversationId]),
  )
  return value === undefined ? null : draftSchema.parse(value)
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

export async function readLocalDrafts(userId: string): Promise<SavedDraft[]> {
  const values = await draftStorage('readonly', (store) =>
    store.getAll(IDBKeyRange.bound([userId], [userId, []])),
  )
  return values
    .map((value) => draftSchema.parse(value))
    .filter((draft) => draft.local !== null)
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
 * A composer's text as the page closes (`web-application.md` § Drafts): the
 * revision it was built on, its Markdown and the files of its marks, which
 * the IndexedDB record carries. An IndexedDB write still under way when the
 * page goes can be lost, so the page writes this where the web browser writes
 * at once, and the next page of the conversation takes it over the record.
 */
const lastWordsSchema = z.object({
  base: z.number().int().nonnegative().nullable(),
  text: z.string(),
  attachmentIds: z.array(z.string()),
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
