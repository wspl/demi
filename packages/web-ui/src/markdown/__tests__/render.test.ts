import { expect, test } from 'bun:test'
import { renderMarkdown, renderUserMarkdown } from '../render'
import type { MessageFiles } from '../types'

// The files a message names (`file-previews.md` § Files named in messages).

const files: MessageFiles = {
  cwd: '/work',
  imageUrl: (path) => `/raw?path=${encodeURIComponent(path)}`,
  open: () => {},
}

test('a link opens a Host file or a web page, and anything else stays text', () => {
  const html = renderMarkdown([
    '[plot](scripts/plot.py:12)',
    '[up](../shared/a.md#usage)',
    '[notes](/home/demi/notes.md)',
    '[windows](C:/Users/zan/a.txt)',
    '[url](file:///tmp/a.txt)',
    '[spaced](out/my%20chart.png)',
    '[web](https://example.com/a)',
    '[section](#install)',
    '[mail](mailto:someone@example.com)',
    '[word](README)',
  ].join('\n\n'), { files })
  expect(html).toContain('<a href="/work/scripts/plot.py" data-file-link>plot</a>')
  expect(html).toContain('<a href="/shared/a.md" data-file-link>up</a>')
  expect(html).toContain('<a href="/home/demi/notes.md" data-file-link>notes</a>')
  expect(html).toContain('<a href="C:/Users/zan/a.txt" data-file-link>windows</a>')
  expect(html).toContain('<a href="/tmp/a.txt" data-file-link>url</a>')
  expect(html).toContain('<a href="/work/out/my chart.png" data-file-link>spaced</a>')
  expect(html).toContain('<a href="https://example.com/a" target="_blank" rel="noopener noreferrer">web</a>')
  for (const text of ['section', 'mail', 'word'])
    expect(html).toContain(`<p>${text}</p>`)
})

test('an image loads from the web, as a data URL, or from the Host; anything else shows its alt text', () => {
  const html = renderMarkdown(
    '![chart](out/chart.png) ![logo](https://example.com/l.png) ![dot](data:image/png;base64,AAAA) ![mail](mailto:a@b.c)',
    { files },
  )
  // A click shows the image whole: a Host file in the File view, a web image in a new tab.
  expect(html).toContain(`<a href="/work/out/chart.png" data-file-link><img src="/raw?path=${encodeURIComponent('/work/out/chart.png')}" alt="chart" /></a>`)
  expect(html).toContain('<a href="https://example.com/l.png" target="_blank" rel="noopener noreferrer"><img src="https://example.com/l.png" alt="logo" /></a>')
  expect(html).toContain(' <img src="data:image/png;base64,AAAA" alt="dot" /> ')
  expect(html).toContain(' mail</p>')
})

test('an image inside a link follows the link', () => {
  expect(renderMarkdown('[![build](https://example.com/badge.svg)](https://example.com/ci)', { files }))
    .toBe('<p><a href="https://example.com/ci" target="_blank" rel="noopener noreferrer"><img src="https://example.com/badge.svg" alt="build" /></a></p>\n')
  expect(renderMarkdown('[![chart](out/chart.png)](docs/charts.md)', { files }))
    .toContain(`<a href="/work/docs/charts.md" data-file-link><img src="/raw?path=${encodeURIComponent('/work/out/chart.png')}" alt="chart" /></a>`)
})

test('without a Host, paths stay text and Host images show their alt text', () => {
  const html = renderMarkdown('[plot](scripts/plot.py) ![chart](out/chart.png) ![logo](https://example.com/l.png)')
  expect(html).not.toContain('data-file-link')
  expect(html).toContain('<p>plot chart <a href="https://example.com/l.png" target="_blank" rel="noopener noreferrer"><img src="https://example.com/l.png" alt="logo" /></a></p>')
})

test('a path in code or plain text is not a link', () => {
  expect(renderMarkdown('See `src/a.ts` and src/b.ts.', { files })).not.toContain('data-file-link')
})

test('task boxes are read-only, and HTML shows as text', () => {
  const html = renderMarkdown('- [x] done\n- [ ] open\n\n<b>bold</b>', { files })
  expect(html).toContain('<input type="checkbox" disabled checked>')
  expect(html).toContain('<input type="checkbox" disabled>')
  expect(html).toContain('&lt;b&gt;bold&lt;/b&gt;')
})

test("a user's message resolves files the same way, without block syntax or math", () => {
  const html = renderUserMarkdown('- not a list\n$x$ and [plot](scripts/plot.py)', { files })
  expect(html).not.toContain('<ul>')
  expect(html).not.toContain('katex')
  expect(html).toContain('- not a list')
  expect(html).toContain('<a href="/work/scripts/plot.py" data-file-link>plot</a>')
})
