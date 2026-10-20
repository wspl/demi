import { afterEach, beforeEach, expect, test } from 'bun:test'
import { forceCloseDatabase, IDBFactory, IDBKeyRange } from 'fake-indexeddb'
import { closeDraftStorage, draftSchema, readDraft, readLocalDrafts, writeDraft, type SavedDraft } from './drafts'
import { newerStorage } from '../state/indexed-db'

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
    comments: [],
    model: { providerId: 'provider', modelId: 'model', thinkingEffort: null, serviceTierId: null },
    files: [],
    scroll: null,
    notSaved: [],
  }
  expect(draftSchema.parse(structuredClone(value))).toEqual(value)
  expect(() => draftSchema.parse({ ...value, messageEdit: { phase: 'sending', request: {} } })).toThrow()
})

// The page's records follow its releases: a record of 0.1.21 is upgraded, and a
// record that still does not read is set aside without costing the others. An
// IndexedDB in memory; milliseconds.
const realIndexedDB = globalThis.indexedDB
const realKeyRange = globalThis.IDBKeyRange
beforeEach(() => {
  globalThis.indexedDB = new IDBFactory()
  globalThis.IDBKeyRange = IDBKeyRange
})
afterEach(() => {
  closeDraftStorage()
  newerStorage.value = null
  globalThis.indexedDB = realIndexedDB
  globalThis.IDBKeyRange = realKeyRange
})

/** Writes `records` into a draft database of version 1, as release 0.1.21 left it. */
async function storeAsRelease0121(records: [key: [string, string], value: unknown][]): Promise<void> {
  await storeAtVersion(1, records)
}

/** Writes `records` into a draft database of `version`. */
async function storeAtVersion(version: number, records: [key: [string, string], value: unknown][]): Promise<void> {
  const request = indexedDB.open('demi-drafts', version)
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
  expect((await readDraft('user', 'sending'))?.pendingSends).toEqual([{ ...sent, comments: [], followers: [] }])
  expect(await readDraft('user', 'idle')).toMatchObject({ pendingSends: [], text: 'typed' })
})

// An edit in progress stored before attachments had numbers names a file by
// its path on the Host, which the backend has since numbered: the edit goes,
// and the draft, its comments included, stays. Before, the whole record was
// set aside, its text with it.
test('a draft of version 3 drops an edit that names a file by its path and keeps the rest', async () => {
  const comment = { id: 'c1', blockId: 'b1', start: 0, end: 4, at: 0, quote: 'Look', text: 'Why?' }
  const record = {
    pendingSends: [], notSaved: [], base: 3, local: null, text: 'typed', comments: [comment], model: null, files: [], scroll: null,
  }
  const edit = {
    phase: 'editing',
    request: {
      operationId: 'op', targetBlockId: 'u1', version: { epoch: 'e', revision: 1 },
      content: [{ type: 'attachment', name: 'shot.png', path: '/home/ana/.demi/attachments/c/shot.png', mediaType: 'image/png', sizeBytes: 4, sha256: 'a'.repeat(64) }],
    },
  }
  await storeAtVersion(3, [[['user', 'edited'], { ...record, messageEdit: edit }]])
  expect(await readDraft('user', 'edited')).toMatchObject({ messageEdit: null, text: 'typed', comments: [comment] })
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

// The web browser can close the page's connection to its database at any
// time: site data cleared, the page frozen and restored, an idle connection
// in a background tab. The next draft write opens a new one rather than
// failing until a reload. An IndexedDB in memory; milliseconds.

const draft = (text: string): SavedDraft => ({
  pendingSends: [], notSaved: [], base: null, local: null, text, comments: [], model: null, files: [], scroll: null,
})

/** The connections the page opens to the draft database, in order. */
function watchConnections(): IDBDatabase[] {
  const opened: IDBDatabase[] = []
  const factory = globalThis.indexedDB
  const open = factory.open.bind(factory)
  factory.open = (name, version) => {
    const request = open(name, version)
    request.addEventListener('success', () => opened.push(request.result))
    return request
  }
  return opened
}

test('a draft saves after the web browser closed the connection and said so', async () => {
  const opened = watchConnections()
  await writeDraft('user', 'conversation', draft('first'))
  // fake-indexeddb declares the parameter as the class rather than an
  // instance of it; the function takes the connection.
  forceCloseDatabase(opened[0] as unknown as typeof IDBDatabase)
  await writeDraft('user', 'conversation', draft('second'))
  expect((await readDraft('user', 'conversation'))?.text).toBe('second')
})

test('a draft saves after the web browser closed the connection without saying so', async () => {
  const opened = watchConnections()
  await writeDraft('user', 'conversation', draft('first'))
  // As Safari does with an idle connection: no close event, and the next
  // transaction on it fails with InvalidStateError.
  opened[0]!.close()
  await writeDraft('user', 'conversation', draft('second'))
  expect((await readDraft('user', 'conversation'))?.text).toBe('second')
})

test('a draft saves when the web browser closed the connection while writing it', async () => {
  const opened = watchConnections()
  await writeDraft('user', 'conversation', draft('first'))
  const connection = opened[0]!
  const transaction = connection.transaction.bind(connection)
  connection.transaction = (stores, mode) => {
    const running = transaction(stores, mode)
    queueMicrotask(() => {
      // A forced close aborts the connection's running transactions with an
      // AbortError, which fake-indexeddb's forceCloseDatabase does not do:
      // its internal abort stands in for it.
      ;(running as unknown as { _abort(name: string): void })._abort('AbortError')
      forceCloseDatabase(connection as unknown as typeof IDBDatabase)
    })
    return running
  }
  await writeDraft('user', 'conversation', draft('second'))
  expect((await readDraft('user', 'conversation'))?.text).toBe('second')
})

test('a page whose draft storage a newer build upgraded asks to load itself again', async () => {
  await writeDraft('user', 'conversation', draft('first'))
  expect(newerStorage.value).toBeNull()
  // A newer build's page in another tab upgrades the database.
  await new Promise((resolve, reject) => {
    const request = indexedDB.open('demi-drafts', 1000)
    request.onsuccess = () => resolve(request.result.close())
    request.onerror = () => reject(request.error)
    request.onblocked = () => reject(new Error('This page kept the draft database open.'))
  })
  await expect(writeDraft('user', 'conversation', draft('second'))).rejects.toMatchObject({ name: 'VersionError' })
  expect(newerStorage.value).not.toBeNull()
})

test('another page deleting the draft database is not blocked, and a draft saves after it', async () => {
  await writeDraft('user', 'conversation', draft('first'))
  await new Promise((resolve, reject) => {
    const request = indexedDB.deleteDatabase('demi-drafts')
    request.onsuccess = resolve
    request.onerror = () => reject(request.error)
    request.onblocked = () => reject(new Error('This page kept the draft database open.'))
  })
  await writeDraft('user', 'conversation', draft('second'))
  expect((await readDraft('user', 'conversation'))?.text).toBe('second')
})
