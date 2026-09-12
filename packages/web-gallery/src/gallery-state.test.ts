import { expect, test } from 'bun:test'
import { galleryStateSchema } from './gallery-state'

const saved = {
  paradigm: 'custom',
  mode: 'dark',
  tone: 'ink',
  accent: 'blue',
  density: 'regular',
  radius: 'medium',
  shadow: 'hairline',
} as const

test('gallery appearance validates the entire snapshot before applying any stored axes', () => {
  expect(galleryStateSchema.parse(saved)).toEqual(saved)
  for (const value of [
    [],
    null,
    {},
    { ...saved, tone: 'unknown' },
    { ...saved, density: [] },
    { ...saved, accent: 'transparent' },
    { ...saved, paradigm: 'deleted' },
  ]) {
    expect(galleryStateSchema.safeParse(value).success).toBe(false)
  }
})
