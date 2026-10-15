import { expect, test } from 'bun:test'
import { createPageI18n, defineText } from '../text'

// A hot reload of a `text.ts` defines its scope again: the new catalogue
// replaces the old one, keys it dropped included. No I/O.
test('a scope defined again replaces its catalogue, and a text read through the earlier handle follows', () => {
  const first = { en: { a: 'one', b: 'two' }, 'zh-Hans': { a: '一', b: '二' }, 'zh-Hant': { a: '一', b: '二' }, ja: { a: '一', b: '二' }, ko: { a: '하나', b: '둘' } }
  const earlier = defineText('test.reload', first)
  expect(earlier.find('b')).toBe('two')
  const next = { en: { a: 'uno' }, 'zh-Hans': { a: '壹' }, 'zh-Hant': { a: '壹' }, ja: { a: '壱' }, ko: { a: '일' } }
  const later = defineText('test.reload', next)
  try {
    expect([earlier.find('a'), later.find('a'), later.find('b')]).toEqual(['uno', 'uno', null])
    createPageI18n('zh-Hans')
    expect(earlier.find('a')).toBe('壹')
  } finally {
    createPageI18n('en')
  }
})
