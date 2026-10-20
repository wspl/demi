import { expect, test } from 'bun:test'
import { filePlace, fileView, firstFileData, locateFolder, type FileData, openFile, shownPath, showFile, showMode, treeShown } from '../data'

// Cost: pure, under a millisecond.

test('each file opened replaces the one shown, in the view chosen last, which the tab records for it', () => {
  const first = showMode(showFile(firstFileData(), 'src/a.md'), 'source')
  expect(first).toEqual({ path: 'src/a.md', view: 'source' })
  // Another file names no view: it shows in the page's choice until the tab records it.
  const second = showFile(first, 'src/b.md')
  expect(second).toEqual({ path: 'src/b.md' })
  // Opening the file shown changes nothing, nor does the view it already shows in.
  expect(showFile(first, 'src/a.md')).toBe(first)
  expect(showMode(first, 'source')).toBe(first)
})

test('the view keeps the directory it was opened for through its files, and another directory moves within the file shown', () => {
  // A child's link opens the file for the child's directory; a pick in its tree stays there.
  const child = showFile(firstFileData(), '/slots/2/src/a.ts', undefined, '/slots/2')
  expect(child).toEqual({ path: '/slots/2/src/a.ts', root: '/slots/2' })
  const picked = showFile(child, '/slots/2/src/b.ts', undefined, child.root)
  expect(picked).toEqual({ path: '/slots/2/src/b.ts', root: '/slots/2' })
  // The conversation's own link names no directory: the view returns to the conversation's.
  expect(showFile(picked, '/work/src/c.ts').root).toBeUndefined()
  // The same file for another directory is not the file shown: the view moves its tree.
  expect(showFile(picked, picked.path, undefined, '/work')).toEqual({ path: picked.path, root: '/work' })
  expect(showFile(picked, picked.path, undefined, '/slots/2')).toBe(picked)
  // A folder shows no file: its tree alone.
  expect(showFile(picked, '', undefined, '/slots/3')).toEqual({ path: '', root: '/slots/3' })
})

// A reload, a copied link and Back show the File view where its address
// says (`docs/decisions/0014-the-work-panel-has-addresses.md`): what the place
// writes reads back as the same view. Planted defect this catches: a field the
// place writes and does not read, such as the directory or the source choice.
test('the File view reads back from its address where it was, and a place that does not read shows its start', () => {
  const views = [
    firstFileData(),
    showFile(firstFileData(), '/work/notes.md'),
    showMode(showFile(firstFileData(), '/slots/2/README.md', 12, '/slots/2'), 'source'),
    showFile(firstFileData(), '', undefined, '/slots/3'),
    locateFolder(showMode(showFile(firstFileData(), '/work/a.ts'), 'preview'), '/work/src'),
  ]
  for (const view of views) {
    // A file is written with the view it shows in: its own, or the page's last choice.
    const shown: FileData = view.path === '' || view.view !== undefined ? view : { ...view, view: 'preview' }
    expect(filePlace.read(filePlace.write(view), firstFileData())).toEqual(shown)
  }
  expect(filePlace.write(views[2]!)).toEqual({ path: '/slots/2/README.md', root: '/slots/2', line: '12', view: 'source' })
  expect(filePlace.read({ line: 'twelve' }, firstFileData())).toBeNull()
  // Show in Files is asked once, of its own file: the address does not hold it.
  const tree = openFile(firstFileData(), { path: '/work/a.ts', tree: true })
  expect(filePlace.read(filePlace.write(tree), tree)).toEqual({ ...tree, view: 'preview' })
  expect(filePlace.read({ path: '/work/b.ts' }, tree)).toEqual({ path: '/work/b.ts' })
  expect(treeShown(tree)).toEqual({ path: '/work/a.ts' })
})

// A file opened without a view shows in the one the user last chose
// anywhere, and the address records it in the same move. Planted defect this
// catches: an address that names no view for it, which a reload shows in the
// page's start.
test('a file opened without a view is written with the one last chosen', () => {
  fileView.value = 'source'
  try {
    expect(filePlace.write(showFile(firstFileData(), '/work/notes.md'))).toEqual({ path: '/work/notes.md', view: 'source' })
    expect(filePlace.write(showMode(showFile(firstFileData(), '/work/notes.md'), 'preview')).view).toBe('preview')
  } finally {
    fileView.value = 'preview'
  }
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
