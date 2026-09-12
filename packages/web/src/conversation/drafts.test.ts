import { expect, test } from 'bun:test'
import { draftSchema } from './drafts'

test('a corrupt visual snapshot is disposable while authored draft fields remain strict', () => {
  const draft = {
    text: 'Keep my draft',
    pendingSend: null,
    local: null,
    model: {
      providerId: '',
      modelId: '',
      thinkingEffort: null,
      serviceTierId: null,
    },
    files: [],
    scroll: { anchor: [], heightCache: {} },
  }
  expect(draftSchema.parse(draft)).toMatchObject({
    text: 'Keep my draft',
    scroll: null,
  })
  expect(draftSchema.safeParse({ ...draft, text: null }).success).toBe(false)
  expect(draftSchema.safeParse({ ...draft, files: [{}] }).success).toBe(false)
})
