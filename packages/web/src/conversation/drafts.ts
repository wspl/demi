import { z } from 'zod'
import { persistedScrollStateSchema } from '@demicodes/web-ui/composables/scroll-state'
import { conversationRecordSchema, hostsSchema } from '@demicodes/product-contracts'
import { editRequestSchema, modelIntentSchema } from '@demicodes/web-ui/transport/protocol'
import type { MessageEditState } from '@demicodes/web-ui/agent/message-editing'
import { displayedUserContentSchema } from '@demicodes/web-ui/transport/protocol'

const messageEditSchema: z.ZodType<MessageEditState> = z.object({
  phase: z.enum(['editing', 'sending', 'uncertain']),
  request: editRequestSchema.extend({ content: z.array(displayedUserContentSchema) }),
})

/** The attachment id the backend returned for the upload. */
const uploadSchema = z.object({ id: z.string() })
const fileSchema = z.object({
  kind: z.literal('file'),
  id: z.string(),
  name: z.string(),
  file: z.instanceof(File),
  upload: uploadSchema.nullable(),
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
  messageEdit: messageEditSchema.nullable().optional(),
  pendingSend: z
    .object({
      id: z.string().uuid(),
      text: z.string(),
      fileIds: z.array(z.string()),
      error: z.string().nullable(),
    })
    .nullable(),
  local: z
    .object({
      phase: z.enum(['draft', 'pending']),
      conversation: conversationRecordSchema.pick({
        id: true,
        title: true,
        pinned: true,
        archived: true,
        target: true,
        createdAt: true,
        updatedAt: true,
      }),
      hosts: z.array(hostsSchema.shape.hosts.element.omit({ online: true })),
    })
    .nullable(),
  text: z.string(),
  model: modelIntentSchema,
  files: z.array(z.union([fileSchema, remoteSchema])),
  // A visual snapshot may be dropped without losing the draft's authored content.
  scroll: persistedScrollStateSchema.nullable().catch(null),
})
export type SavedDraft = z.infer<typeof draftSchema>
export type SavedFile = z.infer<typeof fileSchema>

export class DraftDataError extends Error {
  constructor() {
    super('The saved draft is invalid and has been kept for recovery.')
    this.name = 'DraftDataError'
  }
}

function parseSavedDraft(value: unknown): SavedDraft {
  const result = draftSchema.safeParse(value)
  if (!result.success) {
    throw new DraftDataError()
  }
  return result.data
}

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
        reject(new DOMException('Draft storage closed', 'AbortError'))
        return
      }
      database = db
      db.onversionchange = closeDraftStorage
      resolve(db)
    }
    request.onerror = () => reject(request.error)
    request.onblocked = () => {
      failed = true
      reject(new Error('Draft storage is blocked'))
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
      reject(transaction.error ?? new Error('Draft write aborted'))
  })
}

export async function readDraft(
  userId: string,
  conversationId: string,
): Promise<SavedDraft | null> {
  const value = await draftStorage('readonly', (store) =>
    store.get([userId, conversationId]),
  )
  return value === undefined ? null : parseSavedDraft(value)
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
    .map(parseSavedDraft)
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
