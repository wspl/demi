import { expect, test } from 'bun:test'
import { baseName, clientPlatform, delay, keepComposition, sliceHead, titleCase, truncate } from '../index'

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
  expect(baseName('/Users/zan/Projects/demi')).toBe('demi')
  expect(baseName('/Users/zan/Projects/demi/')).toBe('demi')
  expect(baseName('C:\\Users\\zan\\report.pdf')).toBe('report.pdf')
  expect(baseName('C:\\Users\\zan\\Projects\\')).toBe('Projects')
  expect(baseName('C:/Users/zan/notes.md')).toBe('notes.md')
  expect(baseName('report.pdf')).toBe('report.pdf')
  expect(baseName('/')).toBe('')
})
