import { expect, test } from 'bun:test'
import { draftSchema, type SavedDraft } from './drafts'

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
    pendingSend: null,
    local: null,
    text: 'unrelated composer draft',
    model: { providerId: 'provider', modelId: 'model', thinkingEffort: null, serviceTierId: null },
    files: [],
    scroll: null,
  }
  expect(draftSchema.parse(structuredClone(value))).toEqual(value)
  expect(() => draftSchema.parse({ ...value, messageEdit: { phase: 'sending', request: {} } })).toThrow()
})
