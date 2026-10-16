import { expect, test } from 'bun:test'
import { languageName } from '../../editor/language'
import { fileBar, type ShownFile } from '../file-bar'

// What the File view's bar says and offers for each kind of file (`file-previews.md` § The File view's bars).

/** The bar as the File view draws it, once the editor's language data is there. */
function bar(path: string, shown: ShownFile) {
  return fileBar(path, shown, languageName)
}

const typescript = 'export const a = 1\nexport const b = 2\n'

test('text names its language as the editor knows it, then its lines and size', () => {
  // Lines as GitHub counts them: the line break that ends the last line starts no line.
  expect(bar('/w/src/app.ts', { as: 'text', text: typescript }).label).toBe('TypeScript · 2 lines · 38 B')
  expect(bar('/w/empty.txt', { as: 'text', text: '' }).label).toBe('Plain Text · 0 lines · 0 B')
  expect(bar('/w/blank.txt', { as: 'text', text: '\n' }).label).toBe('Plain Text · 1 line · 1 B')
  // A language the editor ships reads as its language-data name, even for a name the data does not match.
  expect(bar('/w/tsconfig.jsonc', { as: 'text', text: '{}' }).label).toBe('JSON · 1 line · 2 B')
  expect(bar('/w/.gitignore', { as: 'text', text: 'dist\nnode_modules' }).label).toBe('Shell · 2 lines · 17 B')
  expect(bar('/w/notes.txt', { as: 'text', text: 'one\r\ntwo' }).label).toBe('Plain Text · 2 lines · 8 B')
  // An SVG shown as its source is the text the editor colors.
  expect(bar('/w/logo.svg', { as: 'text', text: '<svg/>' }).label).toBe('XML · 1 line · 6 B')
})

test('media name their kind in full and the facts their preview reports, the zoom as it is now', () => {
  const zoomed = bar('/w/shot.png', { as: 'media', facts: { pixels: { width: 1280, height: 720 }, zoom: 1.5, size: 412 * 1024 } })
  expect(zoomed.label).toBe('PNG Image · 1280 × 720 · 150% · 412 KB')
  expect(bar('/w/demo.mp4', { as: 'media', facts: { pixels: { width: 1280, height: 720 }, zoom: null, size: 4.2 * 1024 * 1024 } }).label)
    .toBe('MP4 Video · 1280 × 720 · 4.2 MB')
  expect(bar('/w/voice.m4a', { as: 'media', facts: { pixels: null, zoom: null, size: 3.1 * 1024 * 1024 } }).label).toBe('M4A Audio · 3.1 MB')
  expect(bar('/w/spec.pdf', { as: 'media', facts: { pixels: null, zoom: null, size: 812 * 1024 } }).label).toBe('PDF Document · 812 KB')
  // A video the browser cannot play shows its card, and is still a video.
  expect(bar('/w/demo.mp4', { as: 'card', size: 2048 }).label).toBe('MP4 Video · 2.0 KB')
  expect(bar('/w/app.bin', { as: 'card', size: 2048 }).label).toBe('BIN file · 2.0 KB')
  expect(bar('/w/demo.mp4', { as: 'reading' }).label).toBe('MP4 Video')
})

test('Copy puts the whole text on the clipboard, Markdown its source, and a file shown in part has none', () => {
  const readme = '# Demi\n\n**Bold** words.\n'
  const markdown = bar('/w/README.md', { as: 'text', text: readme })
  expect(markdown).toEqual({ label: 'Markdown · 3 lines · 24 B', modes: true, copy: readme, download: true })
  expect(bar('/w/src/app.ts', { as: 'text', text: typescript })).toMatchObject({ modes: false, copy: typescript })
  // The page holds only the first 8 MiB of a large log: its lines are not known, and Download gives all of it.
  expect(bar('/w/server.log', { as: 'start', size: 12 * 1024 * 1024 }))
    .toEqual({ label: 'Plain Text · 12 MB', modes: false, copy: null, download: true })
  expect(bar('/w/shot.png', { as: 'media', facts: { pixels: null, zoom: null, size: 10 } }).copy).toBeNull()
})

test('a file that is not there names itself and offers nothing', () => {
  expect(bar('/w/TODO.md', { as: 'missing' })).toEqual({ label: 'TODO.md', modes: false, copy: null, download: false })
})
