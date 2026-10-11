import { afterEach, beforeEach, expect, test } from 'bun:test'
import { IDBFactory, IDBKeyRange } from 'fake-indexeddb'
import { closeDraftStorage, draftSchema, readDraft, readLocalDrafts, type SavedDraft } from './drafts'

// Draft storage keeps a structured clone of what a write stores, and a read
// validates it with `draftSchema`: a saved draft comes back whole or not at all.

const SHA256 = 'a'.repeat(64)

test('a saved edit keeps its operation, its files and the separate composer draft through a reload', () => {
  const value: SavedDraft = {
    messageEdit: {
      phase: 'sending',
      request: {
        operationId: crypto.randomUUID(),
        targetBlockId: 'user-1',
        version: { epoch: 'epoch', revision: 2 },
        content: [
          { type: 'text', text: 'Read these' },
          { type: 'document', source: { type: 'ref', ref: SHA256, mediaType: 'application/pdf', fileName: 'kept.pdf' } },
          { type: 'upload', ref: 'attachment-1', fileName: 'added.txt', mediaType: 'text/plain', sha256: SHA256, snippet: 'first line' },
        ],
      },
    },
    pendingSends: [],
    local: null,
    base: 4,
    text: 'unrelated composer draft',
    model: { providerId: 'provider', modelId: 'model', thinkingEffort: null, serviceTierId: null },
    files: [],
    scroll: null,
  }
  expect(draftSchema.parse(structuredClone(value))).toEqual(value)
  expect(() => draftSchema.parse({ ...value, messageEdit: { phase: 'sending', request: {} } })).toThrow()
})

// The page's records follow its releases (`web-application.md` § Persistence
// and adapters): a record of 0.1.21 is upgraded, and a record that still does
// not read is set aside without costing the others. An IndexedDB in memory;
// milliseconds.
const realIndexedDB = globalThis.indexedDB
const realKeyRange = globalThis.IDBKeyRange
beforeEach(() => {
  globalThis.indexedDB = new IDBFactory()
  globalThis.IDBKeyRange = IDBKeyRange
})
afterEach(() => {
  closeDraftStorage()
  globalThis.indexedDB = realIndexedDB
  globalThis.IDBKeyRange = realKeyRange
})

/** Writes `records` into a draft database of version 1, as release 0.1.21 left it. */
async function storeAsRelease0121(records: [key: [string, string], value: unknown][]): Promise<void> {
  const request = indexedDB.open('demi-drafts', 1)
  request.onupgradeneeded = () => request.result.createObjectStore('drafts')
  const db = await new Promise<IDBDatabase>((resolve, reject) => {
    request.onsuccess = () => resolve(request.result)
    request.onerror = () => reject(request.error)
  })
  const transaction = db.transaction('drafts', 'readwrite')
  for (const [key, value] of records) {
    transaction.objectStore('drafts').put(value, key)
  }
  await new Promise((resolve) => {
    transaction.oncomplete = resolve
  })
  db.close()
}

const localConversation = (id: string) => ({
  phase: 'draft' as const,
  conversation: {
    id, title: 'New conversation', pinned: false, archived: false,
    target: { kind: 'cloud' as const }, createdAt: '2026-10-11T00:00:00.000Z', updatedAt: '2026-10-11T00:00:00.000Z',
  },
})

test('a draft of release 0.1.21 reads with its message on its way as a list of one, and one without as an empty list', async () => {
  const sent = { id: crypto.randomUUID(), text: 'Run the tests', fileIds: [], error: null }
  const record = { messageEdit: null, base: 3, local: null, text: '', model: null, files: [], scroll: null }
  await storeAsRelease0121([
    [['user', 'sending'], { ...record, pendingSend: sent }],
    [['user', 'idle'], { ...record, pendingSend: null, text: 'typed' }],
  ])
  expect((await readDraft('user', 'sending'))?.pendingSends).toEqual([sent])
  expect(await readDraft('user', 'idle')).toMatchObject({ pendingSends: [], text: 'typed' })
})

test('a record that does not read is set aside, and every other draft is restored', async () => {
  const record = { messageEdit: null, base: null, text: 'kept', model: null, files: [], scroll: null }
  const [first, broken, third] = [crypto.randomUUID(), crypto.randomUUID(), crypto.randomUUID()]
  await storeAsRelease0121([
    [['user', first], { ...record, pendingSend: null, local: localConversation(first) }],
    [['user', broken], { ...record, pendingSend: null, text: 5, local: localConversation(broken) }],
    [['user', third], { ...record, pendingSend: null, local: localConversation(third) }],
  ])
  const { drafts, unreadable } = await readLocalDrafts('user')
  expect(drafts.map((draft) => draft.local?.conversation.id).sort()).toEqual([first, third].sort())
  expect(unreadable).toHaveLength(1)
  expect(unreadable[0]?.message).toBe('A draft stored in this web browser could not be read.')
  await expect(readDraft('user', broken)).rejects.toThrow('could not be read')
})
