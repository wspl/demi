import { expect, test } from 'bun:test'
import { resolveLanguage } from '../languages'

// System speaks the first of the browser's languages Demi speaks, else
// English (`localization.md` § Choosing the language). A table; no I/O.
test.each([
  { browser: ['zh-TW'], spoken: 'zh-Hant' },
  { browser: ['zh-HK'], spoken: 'zh-Hant' },
  { browser: ['zh-MO'], spoken: 'zh-Hant' },
  { browser: ['zh-Hant'], spoken: 'zh-Hant' },
  { browser: ['zh-Hans-HK'], spoken: 'zh-Hans' },
  { browser: ['zh-CN'], spoken: 'zh-Hans' },
  { browser: ['zh-SG'], spoken: 'zh-Hans' },
  { browser: ['zh'], spoken: 'zh-Hans' },
  { browser: ['en-GB', 'zh-CN'], spoken: 'en' },
  { browser: ['fr', 'ja-JP', 'en'], spoken: 'ja' },
  { browser: ['fr-CA', 'ko-KR'], spoken: 'ko' },
  { browser: ['fr'], spoken: 'en' },
  { browser: ['not a tag', 'ko'], spoken: 'ko' },
  { browser: [], spoken: 'en' },
])('System with $browser speaks $spoken', ({ browser, spoken }) => {
  expect(resolveLanguage('system', browser)).toBe(spoken)
})

test('a chosen language is spoken whatever the browser’s', () => {
  expect(resolveLanguage('ko', ['zh-TW'])).toBe('ko')
})
