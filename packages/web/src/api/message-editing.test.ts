import { afterEach, expect, test } from 'bun:test'
import { loadEditContent } from './message-editing'
import { draftSchema, type SavedDraft } from '../conversation/drafts'

const originalFetch = globalThis.fetch
afterEach(() => { globalThis.fetch = originalFetch })

test('editing hydrates each authenticated attachment by content reference, preserving order and equal filenames', async () => {
  const requests: Array<{ url: string; credentials?: RequestCredentials }> = []
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    requests.push({ url: String(input), credentials: init?.credentials })
    return new Response(new Uint8Array(String(input).endsWith('first') ? [1, 2] : [3, 4]))
  }) as unknown as typeof fetch
  const source = { type: 'ref' as const, fileName: 'same.pdf', mediaType: 'application/pdf' }
  const input = [
    { type: 'text' as const, text: 'first text' },
    { type: 'document' as const, source: { ...source, ref: 'first' } },
    { type: 'text' as const, text: 'last text' },
    { type: 'document' as const, source: { ...source, ref: 'second' } },
  ]
  expect(await loadEditContent(input, new AbortController().signal)).toEqual([
    { type: 'text', text: 'first text' },
    { type: 'document', source: { fileName: 'same.pdf', mediaType: 'application/pdf', data: new Uint8Array([1, 2]) } },
    { type: 'text', text: 'last text' },
    { type: 'document', source: { fileName: 'same.pdf', mediaType: 'application/pdf', data: new Uint8Array([3, 4]) } },
  ])
  expect(requests).toEqual([
    { url: '/api/blobs/first', credentials: 'same-origin' },
    { url: '/api/blobs/second', credentials: 'same-origin' },
  ])
  expect(input[1]!.source).toEqual({ ...source, ref: 'first' })
})

test('failed media hydration preserves the editable blob references', async () => {
  globalThis.fetch = (async () => Response.json({ code: 'unavailable', message: 'try again' }, { status: 503 })) as unknown as typeof fetch
  const input = [{ type: 'image' as const, source: { type: 'ref' as const, ref: 'image', mediaType: 'image/png' } }]
  await expect(loadEditContent(input, new AbortController().signal)).rejects.toThrow('try again')
  expect(input[0]!.source.ref).toBe('image')
})

test('the persisted edit contract keeps operation identity, bytes and the separate composer draft', () => {
  const value: SavedDraft = {
    messageEdit: {
      phase: 'sending', error: null,
      request: {
        operationId: 'operation', targetBlockId: 'user', version: { epoch: 'epoch', revision: 2 },
        content: [{ type: 'document', source: { data: new Uint8Array([9]), fileName: 'a.pdf', mediaType: 'application/pdf' } }],
      },
    },
    pendingSend: null, local: null, text: 'unrelated composer draft',
    model: { providerId: 'p', modelId: 'm', thinkingEffort: null, serviceTierId: null },
    files: [], scroll: null,
  }
  expect(draftSchema.parse(structuredClone(value))).toEqual(value)
  expect(() => draftSchema.parse({ ...value, messageEdit: { ...value.messageEdit, request: {} } })).toThrow()
})
