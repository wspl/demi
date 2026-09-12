import { expect, test } from 'bun:test'
import { readConfiguredModelDraft } from '../model-draft'
import type { SettingsModelDraft } from '../types'

const draft: SettingsModelDraft = {
  id: ' custom ',
  name: '',
  contextWindow: 100,
  outputLimit: 50,
  efforts: [],
  extensions: ['.png', '.mp4'],
  fastTier: null,
}

test('form projection preserves extension capabilities and the id name fallback', () => {
  const result = readConfiguredModelDraft(draft)
  expect(result.success).toBe(true)
  if (!result.success) {
    throw result.error
  }
  expect(result.data).toMatchObject({
    id: 'custom',
    displayName: 'custom',
    acceptedExtensions: ['png', 'mp4'],
  })
})

test('model forms reject values the API cannot store without changing them', () => {
  for (const patch of [
    { contextWindow: 1.5 },
    { contextWindow: null },
    { outputLimit: 101 },
    { extensions: ['.unknown'] },
    { id: ' ' },
  ]) {
    expect(readConfiguredModelDraft({ ...draft, ...patch }).success).toBe(false)
  }
})
