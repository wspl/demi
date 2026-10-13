import { expect, test } from 'bun:test'
import { base64urlDecode, base64urlEncode, baseName, clientPlatform, countOf, countUnlessOne, delay, formatSpan, keepComposition, pluralize, sliceHead, titleCase, truncate } from '../index'

test('an aborted delay ends at once instead of after its time', async () => {
  const stop = new AbortController()
  stop.abort()
  await delay(60_000, stop.signal)
})

test('the platform comes from the web browser that reports it', () => {
  expect(clientPlatform({ platform: 'MacIntel', userAgent: 'Mozilla/5.0 (Macintosh)' })).toBe('mac')
  expect(clientPlatform({ platform: 'Win32', userAgent: 'Mozilla/5.0 (Windows NT 10.0)' })).toBe('windows')
  expect(clientPlatform({ platform: '', userAgent: 'Mozilla/5.0 (X11; Linux x86_64)' })).toBe('linux')
  expect(clientPlatform({ platform: '', userAgent: 'Mozilla/5.0 (SomethingElse)' })).toBe('other')
})

test('a phrase in title case keeps its short inner words lowercase, as the Writing page says', () => {
  expect(titleCase('organize conversations')).toBe('Organize Conversations')
  expect(titleCase('manage skills')).toBe('Manage Skills')
  expect(titleCase('export a document as a file')).toBe('Export a Document as a File')
  expect(titleCase('what to do if it is lost')).toBe('What to Do If It Is Lost')
  expect(titleCase('reach the command-line tool for')).toBe('Reach the Command-Line Tool For')
  expect(titleCase('use built-in tools')).toBe('Use Built-in Tools')
})

test('surrogate-safe slicing', () => {
  // '🙂' is two UTF-16 units.
  expect(sliceHead('a🙂b', 2)).toBe('a')
  expect(sliceHead('a🙂b', 3)).toBe('a🙂')
  expect(sliceHead('a🙂b', 4)).toBe('a🙂b')
  expect(sliceHead('abc', 0)).toBe('')
  expect(truncate('🙂🙂🙂', 4)).toBe('🙂…')
  expect(truncate('🙂🙂🙂', 2, '')).toBe('🙂')
})

test("an input method's keys stay with the field and a plain Enter reaches its handlers", () => {
  const cases = [
    // Chrome: the Enter that commits a candidate, while still composing.
    { key: 'Process', keyCode: 229, isComposing: true, kept: true },
    // Safari: the composition ended before the committing Enter's keydown.
    { key: 'Enter', keyCode: 229, isComposing: false, kept: true },
    // Escape drops a candidate.
    { key: 'Escape', keyCode: 27, isComposing: true, kept: true },
    { key: 'Dead', keyCode: 192, isComposing: false, kept: true },
    { key: 'Enter', keyCode: 13, isComposing: false, kept: false },
    { key: 'Escape', keyCode: 27, isComposing: false, kept: false },
  ]
  for (const { kept, ...key } of cases) {
    let stopped = false
    const answer = keepComposition({ ...key, stopPropagation: () => { stopped = true } })
    expect({ ...key, stopped, answer }).toEqual({ ...key, stopped: kept, answer: kept })
  }
})

test("a Host path's name, on a Mac, Linux or Windows Host", () => {
  expect(baseName('/Users/alex/Projects/demi')).toBe('demi')
  expect(baseName('/Users/alex/Projects/demi/')).toBe('demi')
  expect(baseName('C:\\Users\\zan\\report.pdf')).toBe('report.pdf')
  expect(baseName('C:\\Users\\zan\\Projects\\')).toBe('Projects')
  expect(baseName('C:/Users/alex/notes.md')).toBe('notes.md')
  expect(baseName('report.pdf')).toBe('report.pdf')
  expect(baseName('/')).toBe('')
})

// The duration words of the thinking label, the command time tag, the agent
// status bar and the reset times: each unit floored, the seconds never shown
// from the hour on.
test.each([
  { scenario: 'a moment', ms: 0, wording: '0s' },
  { scenario: 'under a minute', ms: 42_900, wording: '42s' },
  { scenario: 'on the minute', ms: 180_000, wording: '3m' },
  { scenario: 'minutes and seconds', ms: 134_900, wording: '2m 14s' },
  { scenario: 'an hour and seconds', ms: 3_659_000, wording: '1h' },
  { scenario: 'hours and minutes', ms: 3_930_000, wording: '1h 5m' },
  { scenario: 'more than a day', ms: 97_200_000, wording: '27h' },
  { scenario: 'a clock set back', ms: -5_000, wording: '0s' },
])('a span of $scenario reads "$wording"', ({ ms, wording }) => {
  expect(formatSpan(ms)).toBe(wording)
})

// The caller gives the casing: a title passes `Conversation`, a sentence `file`.
test.each([
  { count: 0, pluralized: 'files', counted: '0 files', unlessOne: '0 files' },
  { count: 1, pluralized: 'file', counted: '1 file', unlessOne: 'file' },
  { count: 2, pluralized: 'files', counted: '2 files', unlessOne: '2 files' },
])('$count file(s) read "$counted"', ({ count, pluralized, counted, unlessOne }) => {
  expect(pluralize(count, 'file')).toBe(pluralized)
  expect(countOf(count, 'file')).toBe(counted)
  expect(countUnlessOne(count, 'file')).toBe(unlessOne)
})

test('a count keeps the casing and the irregular plural its caller gives', () => {
  expect(countOf(3, 'Conversation')).toBe('3 Conversations')
  expect(countOf(1, 'Agent')).toBe('1 Agent')
  expect(countOf(2, 'copy', 'copies')).toBe('2 copies')
  expect(pluralize(2, 'This project goes', 'These projects go')).toBe('These projects go')
  expect(pluralize(1, 'This project goes', 'These projects go')).toBe('This project goes')
})

test('base64url carries every byte both ways, without padding, as WebAuthn writes binary values', () => {
  const bytes = new Uint8Array([0, 0xfb, 0xff, 0x3e, 0x3f, 0x80])
  const text = base64urlEncode(bytes)
  expect(text).toBe('APv_Pj-A')
  expect([...base64urlDecode(text)]).toEqual([...bytes])
  // Four bytes take two characters of padding, which the encoding leaves
  // out and the decoding takes either way.
  expect(base64urlEncode(new Uint8Array([0xfb, 0xff, 0x3e, 0x3f]))).toBe('-_8-Pw')
  expect([...base64urlDecode('-_8-Pw==')]).toEqual([0xfb, 0xff, 0x3e, 0x3f])
  expect([...base64urlDecode('-_8-Pw')]).toEqual([0xfb, 0xff, 0x3e, 0x3f])
})
