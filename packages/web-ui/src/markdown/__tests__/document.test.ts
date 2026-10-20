import { expect, test } from 'bun:test'
import { DOCUMENT_RENDER_BYTES, frontMatterAsCode, imageTarget, linkTarget, mediaForTheme, renderable, type DocumentPlace } from '../document'

// Where a Markdown file's links and images lead. The rendering itself runs in a
// web browser and is checked there.

const place: DocumentPlace = {
  path: '/work/docs/guide/README.md',
  root: '/work',
}

test('a link leads to a heading, a web page or a Host path, and to nothing else', () => {
  expect(linkTarget('#getting-started', place)).toBe('#user-content-getting-started')
  expect(linkTarget('#%E5%AE%89%E8%A3%85', place)).toBe('#user-content-安装')
  expect(linkTarget('https://example.com/a?b#c', place)).toBe('https://example.com/a?b#c')
  expect(linkTarget('//cdn.example.com/x', place)).toBe('//cdn.example.com/x')
  expect(linkTarget('install.md#linux', place)).toBe('/work/docs/guide/install.md')
  expect(linkTarget('../api/My%20Notes.md', place)).toBe('/work/docs/api/My Notes.md')
  // A leading slash is the workspace root, as GitHub takes the repository root.
  expect(linkTarget('/CHANGELOG.md', place)).toBe('/work/CHANGELOG.md')
  for (const dropped of ['javascript:alert(1)', 'data:text/html,x', 'mailto:someone@example.com', '#', '?only-a-query'])
    expect(linkTarget(dropped, place)).toBeNull()
})

test('an image loads from the web or from the Host, and from nowhere else', () => {
  expect(imageTarget('logo.png', place)).toEqual({ path: '/work/docs/guide/logo.png' })
  expect(imageTarget('/assets/logo.svg', place)).toEqual({ path: '/work/assets/logo.svg' })
  expect(imageTarget('https://img.shields.io/badge.svg', place)).toEqual({ url: 'https://img.shields.io/badge.svg' })
  for (const dropped of ['data:image/png;base64,AAAA', 'javascript:x', '#top'])
    expect(imageTarget(dropped, place)).toBeNull()
})

test('a source’s color scheme is judged by the theme Demi shows, its other conditions kept', () => {
  const cases: [string, 'light' | 'dark', string | null][] = [
    ['(prefers-color-scheme: dark)', 'dark', 'all'],
    ['(prefers-color-scheme: dark)', 'light', null],
    ['(PREFERS-COLOR-SCHEME:light)', 'light', 'all'],
    ['(prefers-color-scheme: dark) and (min-width: 600px)', 'dark', '(min-width: 0) and (min-width: 600px)'],
    ['(prefers-color-scheme: dark), print', 'light', 'print'],
    ['not (prefers-color-scheme: dark)', 'light', 'all'],
    ['not (prefers-color-scheme: dark)', 'dark', null],
    ['(min-width: 600px)', 'dark', '(min-width: 600px)'],
  ]
  for (const [media, theme, expected] of cases)
    expect({ media, theme, result: mediaForTheme(media, theme) }).toEqual({ media, theme, result: expected })
})

test('leading front matter becomes a YAML code block, fenced past its own backticks', () => {
  expect(frontMatterAsCode('---\ntitle: Guide\ntags: [a, b]\n---\n# Guide\n'))
    .toBe('```yaml\ntitle: Guide\ntags: [a, b]\n```\n# Guide\n')
  expect(frontMatterAsCode('---\nnote: "use ``` fences"\n...\nbody'))
    .toBe('````yaml\nnote: "use ``` fences"\n````\nbody')
  // Only at the very start; a rule later on is a rule.
  expect(frontMatterAsCode('# Title\n---\nx: 1\n---\n')).toBe('# Title\n---\nx: 1\n---\n')
  expect(frontMatterAsCode('---\nunclosed: true\n')).toBe('---\nunclosed: true\n')
})

test('a file over 2 MiB shows only its source, and the limit counts its UTF-8 bytes', () => {
  expect(renderable('a'.repeat(DOCUMENT_RENDER_BYTES))).toBe(true)
  expect(renderable('a'.repeat(DOCUMENT_RENDER_BYTES + 1))).toBe(false)
  // '安' is three UTF-8 bytes, so the limit falls at a third as many characters.
  expect(renderable('安'.repeat(Math.floor(DOCUMENT_RENDER_BYTES / 3)))).toBe(true)
  expect(renderable('安'.repeat(Math.floor(DOCUMENT_RENDER_BYTES / 3) + 1))).toBe(false)
})
