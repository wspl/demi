import { expect, test } from 'bun:test'
import { frontMatterAsCode, imageTarget, linkTarget, type DocumentPlace } from '../document'

// Where a Markdown file's links and images lead (`file-previews.md`
// § Markdown). The rendering itself runs in a browser and is checked there.

const place: DocumentPlace = {
  path: '/work/docs/guide/README.md',
  root: '/work',
  imageUrl: (path) => `/raw?path=${encodeURIComponent(path)}`,
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
  expect(imageTarget('logo.png', place)).toBe(`/raw?path=${encodeURIComponent('/work/docs/guide/logo.png')}`)
  expect(imageTarget('/assets/logo.svg', place)).toBe(`/raw?path=${encodeURIComponent('/work/assets/logo.svg')}`)
  expect(imageTarget('https://img.shields.io/badge.svg', place)).toBe('https://img.shields.io/badge.svg')
  for (const dropped of ['data:image/png;base64,AAAA', 'javascript:x', '#top'])
    expect(imageTarget(dropped, place)).toBeNull()
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
