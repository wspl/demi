import { afterEach, beforeEach, expect, test } from 'bun:test'
import { IDBFactory } from 'fake-indexeddb'
import { waitFor } from '@demicodes/utils'
import { reportError, reportPageErrorsWith, type PageErrorReport } from '@demicodes/web-ui/infra/errors'
import { IndexedDatabase } from './indexed-db'

// Only a defect is reported (`packages/web-ui/src/infra/errors.ts`): a full
// quota is the browser's storage failing, while a write the store refuses,
// as a duplicate key, is the page's own defect. fake-indexeddb stands in for
// the browser's database; a few milliseconds.

const realIndexedDB = globalThis.indexedDB
let reports: PageErrorReport[]
let stop: () => void

beforeEach(() => {
  globalThis.indexedDB = new IDBFactory()
  reports = []
  stop = reportPageErrorsWith(async (report) => {
    reports.push(report)
    return null
  })
})

afterEach(() => {
  stop()
  globalThis.indexedDB = realIndexedDB
})

test('a write the store refuses is reported as a defect, and one over the quota is not', async () => {
  const database = new IndexedDatabase('test', 1, (request) => {
    request.result.createObjectStore('notes')
  })
  await database.transaction('notes', 'readwrite', (store) => store.add('first', 'a'))
  const duplicate = await database.transaction('notes', 'readwrite', (store) => store.add('again', 'a')).catch((error: unknown) => error)
  expect(duplicate).toMatchObject({ name: 'ConstraintError' })
  const full = await database.transaction('notes', 'readwrite', (store) => {
    const request = store.put('more', 'b')
    // The browser aborts a write over the quota with the quota's error.
    Object.defineProperty(store.transaction, 'error', {
      get: () => new DOMException('The quota has been exceeded.', 'QuotaExceededError'),
      set: () => {},
    })
    store.transaction.abort()
    return request
  }).catch((error: unknown) => error)
  expect(full).toMatchObject({ name: 'QuotaExceededError' })
  reportError('Could Not Keep the Copy', duplicate)
  reportError('Could Not Keep the Copy', full)
  // Reports go in order: once this one is in, the others would be too.
  reportError('Could Not Open the File', new Error('handler threw'))
  await waitFor(() => reports.some((report) => report.title === 'Could Not Open the File'))
  expect(reports.map((report) => report.message)).toEqual([String((duplicate as Error).message), 'handler threw'])
  database.close()
})
