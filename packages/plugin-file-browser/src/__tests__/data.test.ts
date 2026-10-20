import { expect, test } from 'bun:test'
import { firstFileData, goBack, goForward, shownPath, showFile } from '../data'

// Cost: pure, under a millisecond.

test('each file opened replaces the one shown and keeps it for Back', () => {
  const second = showFile(showFile(firstFileData(), 'src/a.ts'), 'src/b.ts')
  expect(second).toEqual({ path: 'src/b.ts', back: [{ path: 'src/a.ts' }], forward: [] })
  const back = goBack(second)
  expect(back).toEqual({ path: 'src/a.ts', back: [], forward: [{ path: 'src/b.ts' }] })
  expect(goForward(back)).toMatchObject({ path: 'src/b.ts' })
  const replaced = showFile(back, 'src/c.ts')
  expect(replaced).toMatchObject({ path: 'src/c.ts', forward: [] })
  // Opening the file shown changes nothing.
  expect(showFile(replaced, 'src/c.ts')).toBe(replaced)
})

test('the view keeps the directory it was opened for through its files, and another directory moves within the file shown', () => {
  // A child's link opens the file for the child's directory; a pick in its tree stays there.
  const child = showFile(firstFileData(), '/slots/2/src/a.ts', undefined, '/slots/2')
  expect(child).toEqual({ path: '/slots/2/src/a.ts', root: '/slots/2', back: [], forward: [] })
  const picked = showFile(child, '/slots/2/src/b.ts', undefined, child.root)
  expect(picked).toMatchObject({ path: '/slots/2/src/b.ts', root: '/slots/2', back: [{ path: '/slots/2/src/a.ts', root: '/slots/2' }] })
  // The conversation's own link names no directory: the view returns to the conversation's.
  expect(showFile(picked, '/work/src/c.ts').root).toBeUndefined()
  // The same file for another directory is not the file shown: the view moves its tree, keeping its history.
  const moved = showFile(picked, picked.path, undefined, '/work')
  expect(moved).toMatchObject({ path: picked.path, root: '/work', back: picked.back })
  expect(showFile(picked, picked.path, undefined, '/slots/2')).toBe(picked)
  // A folder shows no file: its tree alone, the file shown kept for Back.
  expect(showFile(picked, '', undefined, '/slots/3')).toMatchObject({
    path: '',
    root: '/slots/3',
    back: [{ path: '/slots/2/src/a.ts', root: '/slots/2' }, { path: '/slots/2/src/b.ts', root: '/slots/2' }],
  })
})

test('Back and Forward show each file under the directory it was opened with', () => {
  // The conversation's file, then a child's folder, then a child's file of another checkout.
  const mine = showFile(firstFileData(), '/work/notes.md')
  const theirs = showFile(mine, '/slots/2/notes.md', undefined, '/slots/2')
  const other = showFile(theirs, '/slots/3/notes.md', undefined, '/slots/3')
  const back = goBack(other)
  expect(back).toMatchObject({ path: '/slots/2/notes.md', root: '/slots/2' })
  const first = goBack(back)
  // The conversation's file returns under the conversation's tree: no directory.
  expect(first.path).toBe('/work/notes.md')
  expect(first.root).toBeUndefined()
  // Forward returns each under its own again, and Back and Forward are inverse.
  expect(goForward(first)).toMatchObject({ path: '/slots/2/notes.md', root: '/slots/2' })
  expect(goForward(goForward(first))).toMatchObject({ path: '/slots/3/notes.md', root: '/slots/3' })
  expect(goForward(goBack(other))).toEqual(other)
})

// An attachment of a Windows Host opens the file it was uploaded from by the
// full path that Host wrote, as a file link names it. Planted defect: a path
// counted as full only when it starts with `/`, which put a Windows one under
// the view's directory.
test('a full path as its Host writes it, a Windows one included, is that file; any other is read from the directory', () => {
  expect(shownPath('C:\\Users\\alex\\shop', 'C:\\Users\\alex\\shop\\out\\chart.png')).toBe('C:\\Users\\alex\\shop\\out\\chart.png')
  expect(shownPath('/work', 'D:/exports/chart.png')).toBe('D:/exports/chart.png')
  expect(shownPath('/work', '\\\\server\\share\\chart.png')).toBe('\\\\server\\share\\chart.png')
  expect(shownPath('/work', '/Users/alex/chart.png')).toBe('/Users/alex/chart.png')
  expect(shownPath('/work', 'out/chart.png')).toBe('/work/out/chart.png')
})
