import { expect, test } from 'bun:test'
import { filePlace, firstFileData, locateFolder, type FileData, openFile, shownPath, showFile, showMode, treeShown } from '../data'

// Cost: pure, under a millisecond.

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
