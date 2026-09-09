import { z } from 'zod'
import { conversationRecordSchema, hostsSchema } from '../api/contracts'

const modelSchema = z.object({
  providerId: z.string(),
  modelId: z.string(),
  thinkingEffort: z.string().nullable(),
  serviceTierId: z.string().nullable(),
})
const uploadSchema = z.discriminatedUnion('kind', [
  z.object({
    kind: z.literal('message'),
    id: z.string(),
    media: z.enum(['image', 'video', 'document']),
  }),
  z.object({
    kind: z.literal('workspace'),
    path: z.string(),
    contextVersion: z.number(),
  }),
])
const fileSchema = z.object({
  kind: z.literal('file'),
  id: z.string(),
  name: z.string(),
  destination: z.enum(['message', 'workspace']),
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
  model: modelSchema,
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
