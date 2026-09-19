import { expect, test } from 'bun:test'
import type { JSONContent } from '@tiptap/core'
import {
  ATTACHMENT_MARK as MARK,
  bareUrls,
  closingFormat,
  parseUserMarkdown,
  serializeUserMarkdown,
} from '../user-markdown'

// A user message's Markdown and the editor document it shows as
// (`product.md` § Writing a message).

function paragraph(...content: JSONContent[]): JSONContent {
  return content.length ? { type: 'paragraph', content } : { type: 'paragraph' }
}

function text(value: string, ...marks: string[]): JSONContent {
  return marks.length
    ? { type: 'text', text: value, marks: marks.map((type) => ({ type })) }
    : { type: 'text', text: value }
}

function doc(...content: JSONContent[]): JSONContent {
  return { type: 'doc', content }
}

function expectRoundTrip(markdown: string, attachmentIds: string[] = []): void {
  expect(serializeUserMarkdown(parseUserMarkdown(markdown, attachmentIds))).toEqual({ markdown, attachmentIds })
}

test('text a conversation about code types stays as typed, both ways', () => {
  for (const markdown of [
    'Rename the session cookie in the login test.',
    'snake_case, __init__, ~/.zshrc, x < y, $PATH, a &amp; b, <b>not html</b>',
    '2 * 3 * 4, *args, **kwargs',
    '# not a heading\n> not a quote\n1. not a list\n- not a list\n---',
    '    indented, and\ttabbed',
    'one\ntwo\n\n\nthree after two blank lines',
    'www.example.com and someone@example.com stay text',
    'C:\\Users\\zan\\a.txt',
    '',
  ]) {
    expectRoundTrip(markdown)
  }
  expect(parseUserMarkdown('snake_case ~/.zshrc <b>x</b>', [])).toEqual(doc(paragraph(text('snake_case ~/.zshrc <b>x</b>'))))
})

test('each line break is kept, and a blank line is an empty line', () => {
  expect(parseUserMarkdown('a\n\nb', [])).toEqual(doc(paragraph(
    text('a'),
    { type: 'hardBreak' },
    { type: 'hardBreak' },
    text('b'),
  )))
})

test('the dialect formats bold, italic, struck and code text, links and images', () => {
  expect(parseUserMarkdown('The **modal** `padding` is *off* ~~now~~.', [])).toEqual(doc(paragraph(
    text('The '),
    text('modal', 'bold'),
    text(' '),
    text('padding', 'code'),
    text(' is '),
    text('off', 'italic'),
    text(' '),
    text('now', 'strike'),
    text('.'),
  )))
  expect(parseUserMarkdown('[plot](scripts/plot.py) ![chart](out/chart.png "Weekly")', [])).toEqual(doc(paragraph(
    { type: 'text', text: 'plot', marks: [{ type: 'link', attrs: { href: 'scripts/plot.py', title: null } }] },
    text(' '),
    { type: 'image', attrs: { src: 'out/chart.png', alt: 'chart', title: 'Weekly' } },
  )))
  for (const markdown of [
    'The **modal** `padding` is *off* ~~now~~.',
    '**bold *and italic* inside**',
    '[plot](scripts/plot.py) ![chart](out/chart.png "Weekly")',
    '[![logo](logo.png)](https://example.com)',
    '[`readSession`](src/session.ts)',
    '``a `tick` inside``',
  ]) {
    expectRoundTrip(markdown)
  }
})

test('bold beside CJK punctuation is bold, as a CJK writer means it', () => {
  expect(parseUserMarkdown('**注意：**这是', [])).toEqual(doc(paragraph(text('注意：', 'bold'), text('这是'))))
})

test('a bare URL is text, which the editor shows as a link', () => {
  const parsed = parseUserMarkdown('see https://example.com/a_b*c.', [])
  expect(parsed).toEqual(doc(paragraph(text('see https://example.com/a_b*c.'))))
  expect(bareUrls('see https://example.com/a_b*c. and http://localhost:3000/x')).toEqual([
    { from: 4, to: 29, href: 'https://example.com/a_b*c' },
    { from: 35, to: 58, href: 'http://localhost:3000/x' },
  ])
})

test('each file is a capsule where its mark is, in mark order', () => {
  const markdown = `Compare ${MARK} with ${MARK}. The **modal** \`padding\` is off.`
  expect(parseUserMarkdown(markdown, ['before', 'after'])).toEqual(doc(paragraph(
    text('Compare '),
    { type: 'attachment', attrs: { id: 'before' } },
    text(' with '),
    { type: 'attachment', attrs: { id: 'after' } },
    text('. The '),
    text('modal', 'bold'),
    text(' '),
    text('padding', 'code'),
    text(' is off.'),
  )))
  expectRoundTrip(markdown, ['before', 'after'])
  expectRoundTrip(`${MARK}${MARK} two files, then\n${MARK} one more`, ['a', 'b', 'c'])
})

test('a file without a mark lands at the end, and a mark without a file is dropped', () => {
  expect(serializeUserMarkdown(parseUserMarkdown('Look', ['a', 'b']))).toEqual({
    markdown: `Look${MARK}${MARK}`,
    attachmentIds: ['a', 'b'],
  })
  expect(serializeUserMarkdown(parseUserMarkdown(`a${MARK}b${MARK}c`, ['one']))).toEqual({
    markdown: `a${MARK}bc`,
    attachmentIds: ['one'],
  })
})

test('formatting does not reach across a capsule or a line break', () => {
  expect(parseUserMarkdown(`**a ${MARK} b**`, ['x'])).toEqual(doc(paragraph(
    text('**a '),
    { type: 'attachment', attrs: { id: 'x' } },
    text(' b**'),
  )))
  expect(serializeUserMarkdown(doc(paragraph(
    text('a', 'bold'),
    { type: 'hardBreak' },
    text('b', 'bold'),
  ))).markdown).toBe('**a**\n**b**')
})

test('a fenced code block keeps its language and its text', () => {
  expectRoundTrip('Why does this throw?\n```ts\nconst a = b*c*d\n```\nThanks.')
  expectRoundTrip('```\n```')
  expectRoundTrip('```\n\n\n```')
  expectRoundTrip('~~~`info` with backticks\ncode\n~~~')
  // The info string is written right after the fence.
  expect(serializeUserMarkdown(parseUserMarkdown('``` ts\nx\n```', [])).markdown).toBe('```ts\nx\n```')
  expect(parseUserMarkdown('```ts\nconst a = 1', [])).toEqual(doc({
    type: 'codeBlock',
    attrs: { language: 'ts' },
    content: [text('const a = 1')],
  }))
  // A fence left open closes once written back.
  expect(serializeUserMarkdown(parseUserMarkdown('```ts\nconst a = 1', [])).markdown).toBe('```ts\nconst a = 1\n```')
  // Code with a fence in it is fenced longer.
  expect(serializeUserMarkdown(doc({
    type: 'codeBlock',
    attrs: { language: 'md' },
    content: [text('```\nx\n```')],
  })).markdown).toBe('````md\n```\nx\n```\n````')
})

test('a mark inside a code block moves its capsule after the block', () => {
  expect(parseUserMarkdown(`\`\`\`\na${MARK}b\n\`\`\``, ['x'])).toEqual(doc(
    { type: 'codeBlock', attrs: { language: null }, content: [text('ab')] },
    paragraph({ type: 'attachment', attrs: { id: 'x' } }),
  ))
})

test('a character typed as itself carries a backslash only where the dialect would read it as syntax', () => {
  const literal = (value: string) => serializeUserMarkdown(doc(paragraph(text(value)))).markdown
  expect(literal('*it*')).toBe('\\*it\\*')
  expect(literal('2 * 3 and *it*')).toBe('2 * 3 and \\*it\\*')
  expect(literal('a `b` c')).toBe('a \\`b\\` c')
  expect(literal('[a](b.md)')).toBe('\\[a\\](b.md)')
  expect(literal('~~a~~ and ~/x')).toBe('\\~\\~a\\~\\~ and ~/x')
  expect(literal('a regex \\*')).toBe('a regex \\\\\\*')
  expect(literal('```')).toBe('\\```')
  expect(literal('https://example.com/~user/*x* and *y*')).toBe('https://example.com/~user/*x* and \\*y\\*')
  for (const value of ['*it*', '2 * 3 and *it*', 'a `b` c', '[a](b.md)', '~~a~~', 'a regex \\*', '```', '~~~ x']) {
    expect(parseUserMarkdown(literal(value), [])).toEqual(doc(paragraph(text(value))))
  }
})

test('formatting written back reads back the same', () => {
  const cases: JSONContent[][] = [
    [text('a', 'bold'), text('b', 'bold', 'italic'), text('c', 'bold')],
    [text('a', 'bold'), text('b', 'italic')],
    [text('a', 'italic'), text('b', 'bold')],
    [text('x'), text('a', 'italic'), text('b')],
    [text('code', 'code', 'bold')],
    [text('a ', 'bold'), text('b')],
    [{ type: 'text', text: 'see the doc', marks: [{ type: 'link', attrs: { href: 'docs/my notes (1).md', title: null } }] }],
    [{ type: 'text', text: 'x', marks: [{ type: 'link', attrs: { href: 'https://a.b/c', title: 'say "hi"' } }, { type: 'code' }] }],
  ]
  for (const nodes of cases) {
    const { markdown } = serializeUserMarkdown(doc(paragraph(...nodes)))
    expect(serializeUserMarkdown(parseUserMarkdown(markdown, [])).markdown).toBe(markdown)
  }
  expect(serializeUserMarkdown(doc(paragraph(text('a ', 'bold'), text('b')))).markdown).toBe('**a** b')
  expect(serializeUserMarkdown(doc(paragraph(text('a', 'bold'), text('b', 'bold', 'italic'), text('c', 'bold')))).markdown)
    .toBe('**a*b*c**')
})

test('typing a closing delimiter formats the construct it closes', () => {
  expect(closingFormat([], 'the *modal*')).toEqual({ length: 7, nodes: [text('modal', 'italic')] })
  expect(closingFormat([], 'a **b**')).toEqual({ length: 5, nodes: [text('b', 'bold')] })
  expect(closingFormat([], 'run `bun test`')).toEqual({ length: 10, nodes: [text('bun test', 'code')] })
  expect(closingFormat([], '[plot](a.py)')).toEqual({
    length: 12,
    nodes: [{ type: 'text', text: 'plot', marks: [{ type: 'link', attrs: { href: 'a.py', title: null } }] }],
  })
  expect(closingFormat([text('注意：', 'bold')], '这是')).toBeNull()
  expect(closingFormat([text('x', 'bold')], '*y*')).toEqual({ length: 3, nodes: [text('y', 'italic')] })
  // Nothing closes: an opening delimiter, a star between spaces, a bare URL,
  // an escape, and bold's first closing star.
  for (const tail of ['a *b', '2 * 3 *', 'see https://example.com', '\\*a*', 'snake_case_', 'The **modal*', 'a ~~b~']) {
    expect(closingFormat([], tail)).toBeNull()
  }
  expect(closingFormat([], 'The **modal**')).toEqual({ length: 9, nodes: [text('modal', 'bold')] })
})
