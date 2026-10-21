import { expect, test } from 'bun:test'
import { resolve } from 'node:path'
import { z } from 'zod'
import { Marked } from 'marked'
import { resetRegionWaits } from '../../infra/region-wait'
import { loadKatex, mathExtensions } from '../math'
import { messageMedia, renderMarkdown } from '../render'
import type { PictureCopy } from '@demicodes/protocol'
import type { AttachmentLookup, MessageFiles, MessagePictures } from '../types'
import type { FileContents } from '../../files/types'
import type { MessageAgents } from '../../agent/agent-links'
import { createPageI18n } from '../../i18n/text'

// The files a message names.

/**
 * The Host's kept files; a message's rendering never reads them.
 */
const contents: FileContents = {
  url: (path) => `/raw?path=${encodeURIComponent(path)}`,
  describe: () => new Promise(() => {}),
  showDescription: () => { throw new Error('Rendering shows no file') },
}

const files: MessageFiles = {
  cwd: '/work',
  contents,
  open: () => {},
}

/**
 * What a reply keeps of its pictures of the Host, as the renderer reads it:
 * the copy of each by the target that names it.
 */
function kept(copies: Record<string, PictureCopy>, streaming = false): MessagePictures {
  return { copies: new Map(Object.entries(copies)), streaming }
}

/**
 * The icon a message's link wears, with the first word of its text, which a
 * line never parts from it.
 */
function lead(icon: string, word: string): string {
  return `<span class="icon-lead"><span class="message-link-icon" ${icon} aria-hidden="true"></span>${word}</span>`
}

/**
 * A link of text to a Host file, as a message shows it: the file's icon, then
 * the text, whose one word is `text`.
 */
function fileLink(path: string, text: string, line?: number): string {
  const name = path.split('/').at(-1)
  return `<a href="${path}" data-file-link${line ? ` data-line="${line}"` : ''} class="icon-link">${lead(`data-file-icon="${name}"`, text)}</a>`
}

const WEB = 'target="_blank" rel="noopener noreferrer nofollow"'

test('a link opens a Host file or a web page, and anything else stays text', () => {
  const html = renderMarkdown([
    '[plot](scripts/plot.py:12)',
    '[up](../shared/a.md#usage)',
    '[notes](/home/demi/notes.md)',
    '[windows](C:/Users/alex/a.txt)',
    '[url](file:///tmp/a.txt)',
    '[spaced](out/my%20chart.png)',
    '[web](https://example.com/a)',
    '[section](#install)',
    '[mail](mailto:someone@example.com)',
    '[word](README)',
  ].join('\n\n'), { files })
  expect(html).toContain(fileLink('/work/scripts/plot.py', 'plot', 12))
  expect(html).toContain(fileLink('/shared/a.md', 'up'))
  expect(html).toContain(fileLink('/home/demi/notes.md', 'notes'))
  expect(html).toContain(fileLink('C:/Users/alex/a.txt', 'windows'))
  expect(html).toContain(fileLink('/tmp/a.txt', 'url'))
  expect(html).toContain(fileLink('/work/out/my chart.png', 'spaced'))
  expect(html).toContain(`<a href="https://example.com/a" ${WEB} class="icon-link">${lead('data-link-icon', 'web')}</a>`)
  for (const text of ['section', 'mail', 'word'])
    expect(html).toContain(`<p>${text}</p>`)
})

test('code in a message\'s link drops code\'s look, its icon already saying what the link is, and code beside it stays code', () => {
  const agents: MessageAgents = {
    find: (number) => (number === 4 ? { number: 4, description: 'File icons', run: null } : null),
    open: () => {},
    stop: () => {},
  }
  const html = renderMarkdown(
    '[`after.png`](out/after.png) [`3f2a9c1`](commit:3f2a9c1) [`docs`](https://example.com) [`icons`](agent:4) `plain`',
    { files: { ...files, openCommit: () => {} }, agents },
  )
  expect(html).toBe('<p>'
    + `${fileLink('/work/out/after.png', 'after.png')} `
    + `<a href="commit:3f2a9c1" data-commit-link="3f2a9c1" class="icon-link">${lead('data-commit-icon', '3f2a9c1')}</a> `
    + `<a href="https://example.com" ${WEB} class="icon-link">${lead('data-link-icon', 'docs')}</a> `
    + '<span role="button" tabindex="0" class="agent-link icon-link" data-agent-link="4" data-keeps-windows><span class="icon-lead"><span class="agent-link-mark" data-agent-mark="4"></span>icons</span></span> '
    + '<code>plain</code></p>\n')
})

test('a commit link starts with the commit icon, spaced as a file link is, and is its text alone when nobody opens commits', () => {
  const html = renderMarkdown('The fix [Fix the total](commit:3f2a9c1); then [again](commit:3f2a9c1) here.', { files: { ...files, openCommit: () => {} } })
  expect(html).toContain(`<a href="commit:3f2a9c1" data-commit-link="3f2a9c1" class="icon-link icon-link-tight">${lead('data-commit-icon', 'Fix')} the total</a>;`)
  expect(html).toContain(`<a href="commit:3f2a9c1" data-commit-link="3f2a9c1" class="icon-link">${lead('data-commit-icon', 'again')}</a>`)
  expect(renderMarkdown('[Fix the total](commit:3f2a9c1)', { files })).toBe('<p>Fix the total</p>\n')
})

test('an image loads from the web, as a data URL, or from the copy a reply keeps of a Host file; anything else shows its alt text', () => {
  const html = renderMarkdown(
    '![chart](out/chart.png) and ![logo](https://example.com/l.png) and ![dot](data:image/png;base64,AAAA) and ![mail](mailto:a@b.c)',
    { files: withAttachments, pictures: kept({ 'out/chart.png': { state: 'copied', attachment: 'a1' } }) },
  )
  // The copy is an attachment's tile; a web image is no tile of one.
  expect(html).toContain(`<p class="media-lone">${hostTile('chart')}</p>`)
  expect(html).toContain('<img src="https://example.com/l.png" alt="logo" />')
  // Each stands on its own line, the words between them on theirs.
  expect(html).toContain('<p>and</p>\n<p class="media-lone"><img src="data:image/png;base64,AAAA" alt="dot" /></p>\n<p>and mail</p>')
})

test('a Host picture with no copy yet is a placeholder, and one that cannot be shown is a link to its file with why', () => {
  const box = '<span class="image-placeholder" role="img" aria-label="chart">chart</span>'
  const link = '<a href="/work/out/chart.png" data-file-link class="icon-link">'
    + `${lead('data-file-icon="chart.png"', 'out')}/chart.png</a>`
  // The reply is arriving, or the backend is copying it.
  expect(renderMarkdown('![chart](out/chart.png)', { files: withAttachments, pictures: kept({}, true) })).toBe(`<p class="media-lone">${box}</p>\n`)
  expect(renderMarkdown('see ![chart](out/chart.png)', { files: withAttachments, pictures: kept({ 'out/chart.png': { state: 'copying' } }) }))
    .toBe(`<p>see</p>\n<p class="media-lone">${box}</p>\n`)
  // The copy's attachment is being asked for.
  expect(renderMarkdown('![chart](out/chart.png)', { files: withAttachments, pictures: kept({ 'out/chart.png': { state: 'copied', attachment: 'a4' } }) }))
    .toBe(`<p class="media-lone">${box}</p>\n`)
  // A failed copy, and a reply that kept no answer, show the file's path as a
  // link; the failure says why.
  const failed = renderMarkdown('see ![chart](out/chart.png)', { files, pictures: kept({ 'out/chart.png': { state: 'failed', reason: { kind: 'missing' } } }) })
  expect(failed).toBe(`<p>see <span class="picture-unavailable">${link} <span class="picture-reason">(The file isn’t on the host)</span></span></p>\n`)
  expect(renderMarkdown('see ![chart](out/chart.png)', { files }))
    .toBe(`<p>see <span class="picture-unavailable">${link}</span></p>\n`)
  // Nobody opens files: the words alone.
  expect(renderMarkdown('see ![chart](out/chart.png)', { files: { ...files, open: undefined } }))
    .toBe('<p>see <span class="picture-unavailable">out/chart.png</span></p>\n')
})

test('a link path starting ~/ names the Host home and stays text while it is not known; a picture of the Host shows its copy either way', () => {
  const markdown = '[todo](~/notes/todo.md) ![chart](~/out/chart.png)'
  const pictures = kept({ '~/out/chart.png': { state: 'copied', attachment: 'a1' } })
  const html = renderMarkdown(markdown, { files: { ...withAttachments, home: '/Users/alex' }, pictures })
  expect(html).toContain(fileLink('/Users/alex/notes/todo.md', 'todo'))
  expect(html).toContain(hostTile('chart'))
  // The copy is the reply's own: the home being unknown, or no Host in reach
  // at all, as on a shared page, changes nothing of it.
  const unknownHome = renderMarkdown(markdown, { files: withAttachments, pictures })
  expect(unknownHome).toContain('<p>todo</p>')
  expect(unknownHome).toContain(hostTile('chart'))
  expect(renderMarkdown('![chart](~/out/chart.png)', { files: { ...withAttachments, open: undefined, cwd: '' }, pictures }))
    .toBe(`<p class="media-lone">${hostTile('chart')}</p>\n`)
})

// Planted defect it catches: a copy looked up by the target as the page's own
// reader leaves it, `out/a&amp;b.png`, which the backend keeps as `out/a&b.png`.
test('a picture is found by its target with its character references resolved, as the backend keeps it', () => {
  const pictures = kept({ 'out/a&b.png': { state: 'copied', attachment: 'a1' } })
  expect(renderMarkdown('![chart](out/a&amp;b.png)', { files: withAttachments, pictures })).toContain(hostTile('chart'))
  expect(messageMedia('![chart](out/a&amp;b.png)', withAttachments, new Map([['out/a&b.png', { state: 'copied', attachment: 'a1' }]])))
    .toHaveLength(1)
})

/**
 * The reading the backend and the page share of a reply's image targets
 * (`crates/agent-transcript/tests/agent-transcript/fixtures/picture-targets.json`):
 * the pictures the backend names are the images the page shows as copies, and
 * no other.
 */
const pictureTargets = z
  .array(z.object({ markdown: z.string(), pictures: z.array(z.string()) }))
  .parse(await Bun.file(resolve(import.meta.dir, '../../../../../crates/agent-transcript/tests/agent-transcript/fixtures/picture-targets.json')).json())

test('the images the backend names as pictures are the ones the page shows as copies', () => {
  const copies = (named: readonly string[]) => kept(Object.fromEntries(named.map((target) => [target, { state: 'copied', attachment: 'a1' } as const])))
  for (const { markdown, pictures } of pictureTargets) {
    const html = renderMarkdown(markdown, { files: { ...withAttachments, home: '/Users/alex' }, pictures: copies(pictures) })
    // Every picture the backend named shows its copy, and with none named the
    // page shows no copy.
    expect(html.includes('data-attachment-image') || html.includes('/blobs/1'), markdown).toBe(pictures.length > 0)
    // Each Host path with no copy is a link or words, never a copy: the
    // backend named no picture the page does not take for a Host file, and the
    // page takes for one every image the backend named.
    expect(html, markdown).not.toContain('picture-unavailable')
  }
})

test('an image inside a link follows the link', () => {
  expect(renderMarkdown('[![build](https://example.com/badge.svg)](https://example.com/ci)', { files }))
    .toBe('<p class="media-lone"><a href="https://example.com/ci" target="_blank" rel="noopener noreferrer nofollow"><img src="https://example.com/badge.svg" alt="build" /></a></p>\n')
  expect(renderMarkdown('[![chart](out/chart.png)](docs/charts.md)', { files: withAttachments, pictures: kept({ 'out/chart.png': { state: 'copied', attachment: 'a1' } }) }))
    .toContain('<a href="/work/docs/charts.md" data-file-link><img src="/blobs/1?type=image%2Fpng" alt="chart" data-width="480" data-height="300" /></a>')
})

test('without a Host, paths stay text and Host images show their alt text', () => {
  const html = renderMarkdown('[plot](scripts/plot.py) ![chart](out/chart.png) ![logo](https://example.com/l.png)')
  expect(html).not.toContain('data-file-link')
  // A Host image's alt text is words; the web image stands on its own line.
  expect(html).toBe('<p>plot chart</p>\n<p class="media-lone"><img src="https://example.com/l.png" alt="logo" /></p>\n')
})

test('emphasis beside CJK text formats as a CJK writer means it', () => {
  // Plain CommonMark reads these stars and tildes, beside CJK punctuation, as
  // text.
  expect(renderMarkdown('**建议：**一组')).toBe('<p><strong>建议：</strong>一组</p>\n')
  expect(renderMarkdown('这是**“扫光”**效果')).toBe('<p>这是<strong>“扫光”</strong>效果</p>\n')
  expect(renderMarkdown('**注意：**これは')).toBe('<p><strong>注意：</strong>これは</p>\n')
  expect(renderMarkdown('**스크립트(script)**는')).toBe('<p><strong>스크립트(script)</strong>는</p>\n')
  expect(renderMarkdown('*强调：*文字 ~~删除：~~文字')).toBe('<p><em>强调：</em>文字 <del>删除：</del>文字</p>\n')
})

test('a link to a line opens the file there', () => {
  expect(renderMarkdown('[runner.md](docs/execution/runner.md#L438) [range](/etc/hosts#L3-L9)', { files }))
    .toBe(`<p>${fileLink('/work/docs/execution/runner.md', 'runner.md', 438)} ${fileLink('/etc/hosts', 'range', 3)}</p>\n`)
})

test('inline code the reply keeps as a file of the Host opens it as a link of words; any other code, and plain text, stay as they are', () => {
  const reply = 'See `src/a.ts:42`, `src/gone.ts`, `README.md`, `runner` and src/b.ts.'
  // A reply that keeps no files, as one from before they were kept, is all code.
  expect(renderMarkdown(reply, { files, codeFiles: null })).not.toContain('data-file-link')
  const html = renderMarkdown(reply, {
    files,
    codeFiles: new Map([
      ['src/a.ts:42', { path: '/work/src/a.ts', line: 42 }],
      ['README.md', { path: '/work/README.md', line: null }],
      ['src/b.ts', { path: '/work/src/b.ts', line: null }],
    ]),
  })
  // A link followed by ASCII punctuation keeps no space before it, as any icon
  // link.
  const a = `<a href="/work/src/a.ts" data-file-link data-line="42" class="icon-link icon-link-tight">${lead('data-file-icon="a.ts"', 'src')}/a.ts:42</a>`
  expect(html).toBe(`<p>See ${a}, <code>src/gone.ts</code>, ${fileLink('/work/README.md', 'README.md').replace('"icon-link"', '"icon-link icon-link-tight"')}, <code>runner</code> and src/b.ts.</p>\n`)
})

test('a link to an agent shows its mark and opens it; a number the conversation does not have is its text', () => {
  const agents: MessageAgents = {
    find: (number) => (number === 4 ? { number: 4, description: 'File icons', run: null } : null),
    open: () => {},
    stop: () => {},
  }
  expect(renderMarkdown('I gave the icons to [File icons](agent:4) and the rest to [Fonts](agent:9).', { files, agents }))
    .toBe('<p>I gave the icons to <span role="button" tabindex="0" class="agent-link icon-link" data-agent-link="4" data-keeps-windows><span class="icon-lead"><span class="agent-link-mark" data-agent-mark="4"></span>File</span> icons</span> and the rest to <span class="agent-link-missing" data-agent-link="9">Fonts</span>.</p>\n')
  // Outside a conversation, the text alone.
  expect(renderMarkdown('[File icons](agent:4)')).toBe('<p>File icons</p>\n')
})

test('a link that starts with an icon keeps no space before ASCII punctuation that follows it, and keeps it before full-width punctuation', () => {
  const agents: MessageAgents = {
    find: (number) => ({ number, description: 'cwd check', run: null }),
    open: () => {},
    stop: () => {},
  }
  const classes = (markdown: string) => [...renderMarkdown(markdown, { files, agents }).matchAll(/class="((?:agent-link )?icon-link[^"]*)"/g)].map((match) => match[1])
  // "cwd check;" reads as written, not "cwd check ;".
  expect(classes('Ask [cwd check](agent:4); then [the notes](notes.md), and [more](agent:5).')).toEqual([
    'agent-link icon-link icon-link-tight', 'icon-link icon-link-tight', 'agent-link icon-link icon-link-tight',
  ])
  // A word after the link, and full-width punctuation, which carries its own
  // space, keep the link's.
  expect(classes('Ask [cwd check](agent:4) first, and 告诉[负责的 Agent](agent:5)：好。')).toEqual([
    'agent-link icon-link', 'agent-link icon-link',
  ])
  // Nested in a list item and a table's cell.
  expect(classes('- see [cwd check](agent:4);\n\n| a |\n| - |\n| [cwd check](agent:4), |')).toEqual([
    'agent-link icon-link icon-link-tight', 'agent-link icon-link icon-link-tight',
  ])
})

test('task boxes are read-only, and HTML shows as text', () => {
  const html = renderMarkdown('- [x] done\n- [ ] open\n\n<b>bold</b>', { files })
  expect(html).toContain('<input type="checkbox" disabled checked>')
  expect(html).toContain('<input type="checkbox" disabled>')
  expect(html).toContain('&lt;b&gt;bold&lt;/b&gt;')
})

// The attachments the agent delivers: `<attachment id="a3" />` alone on its
// line is a block that shows the attachment by what it is.

const attachments: Record<string, AttachmentLookup> = {
  a1: { state: 'found', attachment: { name: 'login.png', mediaType: 'image/png', sha256: '1'.repeat(64), url: '/blobs/1?type=image%2Fpng', size: 15822, width: 480, height: 300 } },
  a2: { state: 'found', attachment: { name: 'flow.mp4', mediaType: 'video/mp4', sha256: '2'.repeat(64), url: '/blobs/2?type=video%2Fmp4', size: 8598311, width: 1280, height: 720 } },
  a3: { state: 'found', attachment: { name: 'report.pdf', mediaType: 'application/pdf', sha256: '3'.repeat(64), url: '/blobs/3?type=application%2Fpdf', size: 2516582 } },
  a4: { state: 'loading' },
  a5: { state: 'failed' },
  a6: { state: 'found', attachment: { name: 'clip.webm', mediaType: 'video/webm', sha256: '6'.repeat(64), url: '/blobs/6?type=video%2Fwebm', size: 23336 } },
  a7: { state: 'found', attachment: { name: 'notes.md', mediaType: 'text/markdown', sha256: '7'.repeat(64), url: '/blobs/7?type=text%2Fmarkdown', size: 496 } },
  a8: { state: 'found', attachment: { name: 'welcome.mp3', mediaType: 'audio/mpeg', sha256: '8'.repeat(64), url: '/blobs/8?type=audio%2Fmpeg', size: 16423 } },
  a45: { state: 'found', attachment: { name: 'words.png', mediaType: 'image/png', sha256: '45'.repeat(32), url: '/blobs/45?type=image%2Fpng', size: 1, width: 2560, height: 1720 } },
  a46: { state: 'found', attachment: { name: 'words-2.png', mediaType: 'image/png', sha256: '46'.repeat(32), url: '/blobs/46?type=image%2Fpng', size: 1, width: 1280, height: 720 } },
  a48: { state: 'found', attachment: { name: 'text.png', mediaType: 'image/png', sha256: '48'.repeat(32), url: '/blobs/48?type=image%2Fpng', size: 1, width: 2560, height: 1720 } },
}
const withAttachments: MessageFiles = {
  ...files,
  attachment: (id) => attachments[id] ?? { state: 'missing' },
}

/** The stored file an attachment's tile carries for a drag into a composer. */
function file(id: string): string {
  const lookup = attachments[id]
  if (lookup?.state !== 'found')
    throw new Error(`No attachment ${id}`)
  const { name, sha256, mediaType } = lookup.attachment
  return ` data-attachment-file="${JSON.stringify({ type: 'stored', name, sha256, mediaType }).replaceAll('"', '&quot;')}"`
}

/** The block that delivers attachment `id`. */
const block = (id: string) => `<attachment id="${id}" />`

const mark = '<span class="media-play-mark" aria-hidden="true"></span>'
/** A picture of the Host a reply shows: the tile of its copy, an attachment's. */
const hostTile = (alt: string) => '<a href="/blobs/1?type=image%2Fpng" data-attachment-image="login.png" data-attachment-id="a1"' + file('a1') + ` data-keeps-windows><img src="/blobs/1?type=image%2Fpng" alt="${alt}" data-width="480" data-height="300" /></a>`
const imageTile = '<a href="/blobs/1?type=image%2Fpng" data-attachment-image="login.png" data-attachment-id="a1"' + file('a1') + ' data-keeps-windows><img src="/blobs/1?type=image%2Fpng" alt="login.png" data-width="480" data-height="300" /></a>'
const videoTile = '<a href="/blobs/2?type=video%2Fmp4" data-attachment-video="flow.mp4" data-attachment-id="a2"' + file('a2') + ' data-keeps-windows class="message-video"><video src="/blobs/2?type=video%2Fmp4" muted playsinline preload="metadata" aria-label="flow.mp4" data-width="1280" data-height="720"></video>' + mark + '</a>'
const card = (id: string) => `<div class="attachment-block" data-attachment-card="${id}"></div>\n`

test('an image block is a tile that a click shows large, and a video block its first frame that a click plays', () => {
  expect(renderMarkdown(block('a1'), { files: withAttachments })).toBe(`<p class="media-lone">${imageTile}</p>\n`)
  expect(renderMarkdown(block('a2'), { files: withAttachments })).toBe(`<p class="media-lone">${videoTile}</p>\n`)
  // A record without a size leaves it to the first frame.
  expect(renderMarkdown('<attachment id="a6"/>', { files: withAttachments }))
    .toBe(`<p class="media-lone"><a href="/blobs/6?type=video%2Fwebm" data-attachment-video="clip.webm" data-attachment-id="a6"${file('a6')} data-keeps-windows class="message-video"><video src="/blobs/6?type=video%2Fwebm" muted playsinline preload="metadata" aria-label="clip.webm"></video>${mark}</a></p>\n`)
})

test('an audio block is its inline player, and any other file\'s block the place of its card', () => {
  expect(renderMarkdown(block('a8'), { files: withAttachments }))
    .toBe('<p><span class="message-audio" data-attachment-audio="/blobs/8?type=audio%2Fmpeg" data-audio-name="welcome.mp3"></span></p>\n')
  expect(renderMarkdown(block('a3'), { files: withAttachments })).toBe(card('a3'))
  // Cards on consecutive lines stack, each its own; images that follow each
  // other stand side by side.
  expect(renderMarkdown(`${block('a3')}\n${block('a7')}`, { files: withAttachments })).toBe(`${card('a3')}${card('a7')}`)
  expect(renderMarkdown(`${block('a1')}\n${block('a2')}`, { files: withAttachments })).toBe(`<p class="media-run">${imageTile}${videoTile}</p>\n`)
})

test('a block on the line after words, or in a list item, stands on a line of its own', () => {
  expect(renderMarkdown(`The report:\n${block('a3')}\nThat is all.`, { files: withAttachments }))
    .toBe(`<p>The report:</p>\n${card('a3')}<p>That is all.</p>\n`)
  expect(renderMarkdown(`- The report\n  ${block('a3')}`, { files: withAttachments }))
    .toBe(`<ul>\n<li>The report${card('a3')}</li>\n</ul>\n`)
})

test('the tag in a sentence, and a link or an image to an attachment, are text', () => {
  expect(renderMarkdown(`See ${block('a3')} for it.`, { files: withAttachments }))
    .toBe('<p>See &lt;attachment id=&quot;a3&quot; /&gt; for it.</p>\n')
  expect(renderMarkdown('[the report](attachment:a3) and ![the page](attachment:a1)', { files: withAttachments }))
    .toBe('<p>the report and the page</p>\n')
  // In code it is code.
  expect(renderMarkdown(`\`\`\`\n${block('a3')}\n\`\`\``, { files: withAttachments })).not.toContain('data-attachment-card')
})

// What is a block is one rule the backend follows too, to know which files a
// share serves: both read the same cases, so they cannot drift. Each attachment
// is a file, so a block is a card.
const blockCases = z.array(z.strictObject({ case: z.string(), text: z.string(), blocks: z.array(z.string()) }))
  .parse(await Bun.file(resolve(import.meta.dir, '../../../../../crates/shared-types/tests/shared-types/fixtures/attachment-blocks.json')).json())
const everyFile: MessageFiles = {
  ...files,
  attachment: (id) => ({ state: 'found', attachment: { name: `${id}.pdf`, mediaType: 'application/pdf', sha256: '0'.repeat(64), url: `/blobs/${id}`, size: 1 } }),
}

test.each(blockCases.map((entry) => [entry.case, entry] as const))('a block, or text: %s', (_case, { text, blocks }) => {
  const html = renderMarkdown(text, { files: everyFile })
  expect([...html.matchAll(/data-attachment-card="(a\d+)"/g)].map((found) => found[1])).toEqual(blocks)
})

test('a block says so of a number the conversation does not have, keeps its card\'s place while the page asks, and is its text when it cannot ask', () => {
  expect(renderMarkdown(block('a9'), { files: withAttachments })).toBe('<p class="attachment-missing">No attachment a9 in this conversation</p>\n')
  expect(renderMarkdown(block('a4'), { files: withAttachments })).toBe(card('a4'))
  const text = '<p>&lt;attachment id=&quot;a5&quot; /&gt;</p>\n'
  expect(renderMarkdown(block('a5'), { files: withAttachments })).toBe(text)
  expect(renderMarkdown(block('a5'))).toBe(text)
})

test('a block of a number the conversation does not have says so in the page\'s language', () => {
  createPageI18n('zh-Hans')
  try {
    expect(renderMarkdown(block('a9'), { files: withAttachments })).toBe('<p class="attachment-missing">此对话中没有附件 a9</p>\n')
  } finally {
    createPageI18n('en')
  }
})

// Images that follow each other stand side by side: a run is one row, each
// image an item of it.

const shot = (n: number) => `![shot ${n}](https://example.com/${n}.png)`
const item = (n: number) => `<img src="https://example.com/${n}.png" alt="shot ${n}" />`

test('images with only white space between them in one paragraph are one run', () => {
  expect(renderMarkdown(`${shot(1)} ${shot(2)}\n${shot(3)}`, { files }))
    .toBe(`<p class="media-run">${item(1)}${item(2)}${item(3)}</p>\n`)
})

test('paragraphs of images that follow each other are one run, a video’s block among them', () => {
  expect(renderMarkdown(`${shot(1)}\n\n${block('a2')}\n\n\n${shot(2)}`, { files: withAttachments }))
    .toBe(`<p class="media-run">${item(1)}${videoTile}${item(2)}</p>\n`)
})

test('text, a list or a heading between images ends the run', () => {
  const html = renderMarkdown(
    `${shot(1)}\n${shot(2)}\n\nThen:\n\n${shot(3)}\n${shot(4)}\n\n- one\n\n${shot(5)} and ${shot(6)}\n\n# Done\n\n${shot(7)}`,
    { files },
  )
  expect(html.match(/class="media-run"/g)).toHaveLength(2)
  expect(html).toContain(`<p class="media-run">${item(1)}${item(2)}</p>`)
  expect(html).toContain(`<p class="media-run">${item(3)}${item(4)}</p>`)
  expect(html).toContain(`<p class="media-lone">${item(5)}</p>`)
  expect(html).toContain(`<p class="media-lone">${item(7)}</p>`)
})

const lone = (n: number) => `<p class="media-lone"><img src="https://example.com/${n}.png" alt="shot ${n}" /></p>\n`

test('a lone image stands on a line of its own, even in a paragraph of words', () => {
  expect(renderMarkdown(`Here it is:\n\n${shot(1)}\n\nThat is all.`, { files }))
    .toBe(`<p>Here it is:</p>\n${lone(1)}<p>That is all.</p>\n`)
  expect(renderMarkdown(`Here it is:\n${shot(1)}\nThat is all.`, { files }))
    .toBe(`<p>Here it is:</p>\n${lone(1)}<p>That is all.</p>\n`)
})

// The reply the user saw: a list item's words, then a screenshot on the next
// line, showed it in the item's line at the message's full width.
test('an image on the line after a list item’s words stands on its own line under them', () => {
  expect(renderMarkdown('- **Text** ... (this one is 17 px).\n  <attachment id="a48" />\n- Next', { files: withAttachments }))
    .toBe(`<ul>\n<li><strong>Text</strong> ... (this one is 17 px).<p class="media-lone"><a href="/blobs/48?type=image%2Fpng" data-attachment-image="text.png" data-attachment-id="a48"${file('a48')} data-keeps-windows><img src="/blobs/48?type=image%2Fpng" alt="text.png" data-width="2560" data-height="1720" /></a></p>\n</li>\n<li>Next</li>\n</ul>\n`)
})

// The reply the user saw: a list item's words, then two screenshots one per
// line, showed each at full width under the other instead of side by side.
test('images after a list item’s words stand side by side under them', () => {
  const html = renderMarkdown('- **Words** ... text.\n  <attachment id="a45" />\n  <attachment id="a46" />', { files: withAttachments })
  expect(html).toStartWith('<ul>\n<li><strong>Words</strong> ... text.<p class="media-run"><a href="/blobs/45?type=image%2Fpng"')
  expect(html.match(/<img /g)).toHaveLength(2)
  expect(html).toContain('alt="words-2.png" data-width="1280" data-height="720" /></a></p>\n</li>')
})

test('an image inside a link counts as an image of a run, and follows its link', () => {
  expect(renderMarkdown(`[![build](https://example.com/badge.svg)](https://example.com/ci)\n${shot(1)}`, { files }))
    .toBe(`<p class="media-run"><a href="https://example.com/ci" target="_blank" rel="noopener noreferrer nofollow"><img src="https://example.com/badge.svg" alt="build" /></a>${item(1)}</p>\n`)
})

// Dollar signs follow Pandoc's rule; a file renders math through the same
// syntax.

test('prices stay text', () => {
  // A closing `$` needs no space before it and no digit after it.
  for (const text of ['Notion is $10 vs $12; Slack $7.25 vs $8.75', 'Plans cost $5/$6 a month'])
    expect(renderMarkdown(text)).toBe(`<p>${text}</p>\n`)
})

test('until KaTeX arrives, math shows its source where it will stand, hidden at first, and asks for KaTeX', () => {
  // A KaTeX that has not arrived, whatever another test loaded in this process,
  // in the 300 ms it is waited for (`region-draws-once.test.ts`).
  resetRegionWaits()
  let asked = 0
  const reader = new Marked({ extensions: mathExtensions({}, () => {
    asked += 1
    return null
  }) })
  expect(reader.parse('面积为$x^2$平方米')).toBe('<p>面积为<span class="math math-inline math-waiting">x^2</span>平方米</p>\n')
  expect(reader.parse('$$a < b$$')).toBe('<p><span class="math math-display math-waiting">a &lt; b</span></p>\n')
  expect(reader.parse('$$\nx_1\n$$')).toBe('<span class="math math-display math-waiting">x_1</span>\n')
  expect(asked).toBeGreaterThan(0)
})

test('math renders on its own, beside Chinese text, and as a display', async () => {
  await loadKatex()
  expect(renderMarkdown('$x$')).toStartWith('<p><span class="katex">')
  const beside = renderMarkdown('面积为$x^2$平方米')
  expect(beside).toStartWith('<p>面积为<span class="katex">')
  expect(beside).toEndWith('</span>平方米</p>\n')
  expect(renderMarkdown('$$\\sum_i x_i$$')).toContain('<span class="katex-display">')
})

// The pictures a message shows that open in the media viewer, in order.

test('a message\'s pictures are its images that load and its attachment blocks that are images or videos, in order', () => {
  const markdown = [
    '![chart](out/chart.png) and ![logo](https://example.com/l.png)',
    block('a2'),
    `![mail](mailto:a@b.c) ![page](attachment:a1) ${block('a1')}`,
    [block('a3'), block('a4'), block('a9')].join('\n'),
    '`![code](not-an-image.png)`',
    block('a1'),
  ].join('\n\n')
  const pictures = new Map<string, PictureCopy>([['out/chart.png', { state: 'copied', attachment: 'a1' }]])
  expect(messageMedia(markdown, withAttachments, pictures)).toEqual([
    { kind: 'image', src: '/blobs/1?type=image%2Fpng', name: 'login.png', size: { width: 480, height: 300 }, bytes: 15822 },
    { kind: 'image', src: 'https://example.com/l.png', name: 'logo' },
    { kind: 'video', src: '/blobs/2?type=video%2Fmp4', name: 'flow.mp4', size: { width: 1280, height: 720 }, bytes: 8598311 },
    { kind: 'image', src: '/blobs/1?type=image%2Fpng', name: 'login.png', size: { width: 480, height: 300 }, bytes: 15822 },
  ])
})

test('an image inside a link follows the link and is not among a message\'s pictures', () => {
  const pictures = new Map<string, PictureCopy>([['out/chart.png', { state: 'copied', attachment: 'a1' }]])
  expect(messageMedia('[![build](https://example.com/badge.svg)](https://example.com/ci) ![chart](out/chart.png)', withAttachments, pictures))
    .toEqual([{ kind: 'image', src: '/blobs/1?type=image%2Fpng', name: 'login.png', size: { width: 480, height: 300 }, bytes: 15822 }])
  // A picture with no copy in, or that failed, is none of them.
  expect(messageMedia('![chart](out/chart.png)', withAttachments, new Map([['out/chart.png', { state: 'copying' }]]))).toEqual([])
  expect(messageMedia('![chart](out/chart.png)', withAttachments, new Map())).toEqual([])
})
