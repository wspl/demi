import { describe, expect, test } from 'bun:test'
import { nextTick } from 'vue'
import { HostFiles } from '../file-cache'
import { keptSource } from '../kept-source'
import { completionSpot, rankCompletions, usePathCompletion, type PathCompletionKind } from '../path-completion'
import type { FileBrowserEntry } from '../types'

const home = '/Users/zan'

describe('the spot the caret stands at', () => {
  test('the text before the caret splits at its last slash into the directory and the query', () => {
    expect(completionSpot('/Users/zan/Pro', 14, { home })).toEqual({ directory: '/Users/zan', query: 'Pro', start: 11 })
    // Right after a slash: that directory, unfiltered.
    expect(completionSpot('/Users/zan/', 11, { home })).toEqual({ directory: '/Users/zan', query: '', start: 11 })
    // What follows the caret plays no part.
    expect(completionSpot('/Users/zan/Projects/demi', 13, { home })).toEqual({ directory: '/Users/zan', query: 'Pr', start: 11 })
    expect(completionSpot('/', 1, { home })).toEqual({ directory: '/', query: '', start: 1 })
  })

  test('~ starts at the home, and a relative path from the base or nowhere', () => {
    expect(completionSpot('~/Do', 4, { home })).toEqual({ directory: '/Users/zan', query: 'Do', start: 2 })
    expect(completionSpot('src/fi', 6, { home, base: '/w' })).toEqual({ directory: '/w/src', query: 'fi', start: 4 })
    expect(completionSpot('READ', 4, { home, base: '/w' })).toEqual({ directory: '/w', query: 'READ', start: 0 })
    expect(completionSpot('src/fi', 6, { home })).toBeNull()
    // An absolute path ignores the base.
    expect(completionSpot('/etc/ho', 7, { home, base: '/w' })).toEqual({ directory: '/etc', query: 'ho', start: 5 })
  })
})

const entry = (name: string, isDirectory = false): FileBrowserEntry => ({ name, isDirectory })
const names = (query: string, kind: PathCompletionKind = 'any', entries = listing) =>
  rankCompletions(entries, query, kind).map((row) => row.entry.name)

const listing = [
  entry('a_b_c.txt'),
  entry('abc', true),
  entry('abc.md'),
  entry('.abc'),
  entry('.config', true),
  entry('Projects', true),
  entry('zeta.txt'),
]

describe('ranking a directory\'s entries', () => {
  test('the match is fuzzy and ignores case, its letters given for marking', () => {
    expect(names('abc')).toContain('a_b_c.txt')
    expect(names('PRO')).toEqual(['Projects'])
    const fuzzy = rankCompletions(listing, 'abc', 'any').find((row) => row.entry.name === 'a_b_c.txt')
    expect(fuzzy?.indexes).toEqual([0, 2, 4])
  })

  test('better matches come first; equal ones put directories first, then names in order', () => {
    // `abc` and `abc.md` start with the whole query; the scattered match comes last.
    expect(names('abc')).toEqual(['abc', 'abc.md', 'a_b_c.txt'])
    // An empty query matches all alike: the file browser's order.
    expect(names('')).toEqual(['abc', 'Projects', 'a_b_c.txt', 'abc.md', 'zeta.txt'])
  })

  test('dot entries show only to a query that starts with a dot', () => {
    expect(names('c')).not.toContain('.config')
    expect(names('.c')).toEqual(['.config', '.abc'])
  })

  test('a directory field offers no files', () => {
    expect(names('', 'directory')).toEqual(['abc', 'Projects'])
  })
})

/**
 * A kept source over fixed directories under a live watch, counting each
 * listing, whose listings land when the test says.
 */
function fakeSource(tree: Record<string, FileBrowserEntry[]>) {
  const calls: { path: string }[] = []
  const pending: (() => void)[] = []
  const files = new HostFiles()
  files.cover({ covers: () => true })
  const source = keptSource({
    platform: 'macos',
    home,
    list(path: string): Promise<FileBrowserEntry[]> {
      calls.push({ path })
      return new Promise((resolve, reject) => {
        pending.push(() => {
          const entries = tree[path]
          if (entries)
            resolve(entries)
          else
            reject(new Error('not found'))
        })
      })
    },
  }, { files })
  /** Answers every listing asked for so far, and lets their results land. */
  async function answer(): Promise<void> {
    // The field shows the directory once its watcher has run.
    await nextTick()
    for (const settle of pending.splice(0))
      settle()
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
  return { source, calls, answer }
}

const tree = {
  '/Users/zan': [entry('Documents', true), entry('Projects', true), entry('notes.md')],
  '/Users/zan/Projects': [entry('demi', true), entry('demo.txt')],
}

describe('a path field\'s menu', () => {
  test('accepting a directory writes its name and a slash, and the menu lists it next', async () => {
    const { source, calls, answer } = fakeSource(tree)
    const completion = usePathCompletion({ source: () => source, base: () => undefined, kind: () => 'any' })
    const text = '/Users/zan/Pro/rest'
    completion.follow(text, 14)
    await answer()
    expect(completion.rows.value.map((row) => row.entry.name)).toEqual(['Projects'])
    // The list opens with its first row selected, which Tab takes; what follows the caret stays.
    const accepted = completion.keydown('Tab', text, 14)
    expect(accepted).toEqual({
      kind: 'accept',
      edit: { text: '/Users/zan/Projects//rest', caret: 20 },
      entry: entry('Projects', true),
    })
    if (accepted.kind !== 'accept')
      return
    completion.follow(accepted.edit.text, accepted.edit.caret)
    await answer()
    expect(calls.map((call) => call.path)).toEqual(['/Users/zan', '/Users/zan/Projects'])
    expect(completion.rows.value.map((row) => row.entry.name)).toEqual(['demi', 'demo.txt'])
    expect(completion.isOpen.value).toBe(true)
  })

  test('the first row is selected as the list opens and as its rows change, Tab takes the selected row, and Return is the field\'s', async () => {
    const { source, answer } = fakeSource(tree)
    const completion = usePathCompletion({ source: () => source, base: () => undefined, kind: () => 'any' })
    const text = '/Users/zan/'
    completion.follow(text, 11)
    await answer()
    expect(completion.rows.value.map((row) => row.entry.name)).toEqual(['Documents', 'Projects', 'notes.md'])
    expect(completion.highlighted.value).toBe(0)
    expect(completion.keydown('ArrowDown', text, 11)).toEqual({ kind: 'handled' })
    expect(completion.keydown('Tab', text, 11)).toMatchObject({ kind: 'accept', edit: { text: '/Users/zan/Projects/' } })
    // Return submits what the field holds, as the form's or the bar's own key.
    expect(completion.keydown('Enter', text, 11)).toEqual({ kind: 'pass' })
    expect(completion.keydown('ArrowUp', text, 11)).toEqual({ kind: 'handled' })
    expect(completion.keydown('ArrowUp', text, 11)).toEqual({ kind: 'handled' })
    expect(completion.keydown('Tab', text, 11)).toMatchObject({ kind: 'accept', edit: { text: '/Users/zan/notes.md' } })
    // The pointer moves the selection too.
    completion.highlight(1)
    expect(completion.keydown('Tab', text, 11)).toMatchObject({ kind: 'accept', edit: { text: '/Users/zan/Projects/' } })
    // Typing changes the rows, and the first of them is selected again.
    completion.follow('/Users/zan/no', 13)
    expect(completion.rows.value.map((row) => row.entry.name)).toEqual(['notes.md'])
    expect(completion.highlighted.value).toBe(0)
    expect(completion.keydown('Tab', '/Users/zan/no', 13)).toMatchObject({ kind: 'accept', edit: { text: '/Users/zan/notes.md' } })
  })

  test('with no list showing, Tab is the field\'s', async () => {
    const { source, answer } = fakeSource(tree)
    const completion = usePathCompletion({ source: () => source, base: () => undefined, kind: () => 'any' })
    // A query that matches nothing shows no list.
    completion.follow('/Users/zan/xyz', 14)
    await answer()
    expect(completion.isOpen.value).toBe(false)
    expect(completion.keydown('Tab', '/Users/zan/xyz', 14)).toEqual({ kind: 'pass' })
    // A list the user put away with Escape leaves Tab to the field.
    completion.follow('/Users/zan/', 11)
    await answer()
    expect(completion.keydown('Escape', '/Users/zan/', 11)).toEqual({ kind: 'handled' })
    expect(completion.keydown('Tab', '/Users/zan/', 11)).toEqual({ kind: 'pass' })
  })

  test('Escape puts the menu away, and the next Escape is the field\'s', async () => {
    const { source, answer } = fakeSource(tree)
    const completion = usePathCompletion({ source: () => source, base: () => undefined, kind: () => 'any' })
    completion.follow('/Users/zan/', 11)
    await answer()
    expect(completion.keydown('Escape', '/Users/zan/', 11)).toEqual({ kind: 'handled' })
    expect(completion.isOpen.value).toBe(false)
    expect(completion.keydown('Escape', '/Users/zan/', 11)).toEqual({ kind: 'pass' })
    // Typing on brings it back.
    completion.follow('/Users/zan/D', 12)
    await answer()
    expect(completion.isOpen.value).toBe(true)
  })

  test('a directory listed while the Host confirms it asks nothing again, and a failed one shows nothing', async () => {
    const { source, calls, answer } = fakeSource(tree)
    const completion = usePathCompletion({ source: () => source, base: () => undefined, kind: () => 'any' })
    completion.follow('/Users/zan/', 11)
    // The caret moves on before the listing lands.
    completion.follow('/Users/zan/Projects/', 20)
    await answer()
    // Back and forth between listed directories asks for nothing again.
    completion.follow('/Users/zan/', 11)
    await answer()
    completion.follow('/Users/zan/Projects/d', 21)
    completion.follow('/Users/zan/Pr', 13)
    await answer()
    expect(calls.map((call) => call.path)).toEqual(['/Users/zan/Projects', '/Users/zan'])
    expect(completion.rows.value.map((row) => row.entry.name)).toEqual(['Projects'])
    completion.follow('/nowhere/', 9)
    await answer()
    expect(completion.rows.value).toEqual([])
    expect(completion.isOpen.value).toBe(false)
  })
})
