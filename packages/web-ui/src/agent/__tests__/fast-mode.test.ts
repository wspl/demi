import { expect, test } from 'bun:test'
import type { ModelInfo } from '../../transport/protocol'
import { fastServiceTier, isFastMode } from '../fast-mode'

const model: ModelInfo = {
  id: 'm',
  name: 'M',
  contextWindow: 200000,
  inputLimit: null,
  acceptedExtensions: [],
  reasoning: null,
  serviceTiers: [
    { id: 'flex', label: 'Flex', fast: false },
    { id: 'priority', label: 'Fast', fast: true },
  ],
}

test('the Fast tier is the one the provider flagged, whatever it is called', () => {
  expect(fastServiceTier(model)?.id).toBe('priority')
  expect(fastServiceTier({ ...model, serviceTiers: [{ id: 'turbo', label: 'Fastest available', fast: true }] })?.id).toBe('turbo')
})

test('a model without a Fast tier has no Fast Mode', () => {
  expect(fastServiceTier({ ...model, serviceTiers: null })).toBeNull()
  expect(fastServiceTier({ ...model, serviceTiers: [{ id: 'flex', label: 'Flex', fast: false }] })).toBeNull()
  expect(isFastMode({ ...model, serviceTiers: null }, 'priority')).toBe(false)
})

test('Fast Mode is on only when the session tier is the Fast tier', () => {
  expect(isFastMode(model, 'priority')).toBe(true)
  expect(isFastMode(model, 'flex')).toBe(false)
  expect(isFastMode(model, null)).toBe(false)
})
