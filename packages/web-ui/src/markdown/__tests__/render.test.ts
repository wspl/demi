import { expect, test } from 'bun:test'
import { messageMedia, renderMarkdown } from '../render'
import type { AttachmentLookup, MessageFiles } from '../types'
import type { HostImage } from '../../files/host-image'
import type { FileContents } from '../../files/types'
import type { MessageAgents } from '../../agent/agent-links'

// The files a message names (`file-previews.md` § Files named in messages).

/** The Host's kept files, which a message's Host images show; never read here. */
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

/** The Host images at `paths` as the page shows them once their files are read. */
function shownImages(...paths: string[]): ReadonlyMap<string, HostImage> {
  return new Map(paths.map((path) => [path, { state: 'shown', src: `/raw?path=${encodeURIComponent(path)}` }]))
}

/** The icon a message's link wears, with the first word of its text, which a line never parts from it. */
function lead(icon: string, word: string): string {
  return `<span class="icon-lead"><span class="message-link-icon" ${icon} aria-hidden="true"></span>${word}</span>`
}

/** A link of text to a Host file, as a message shows it: the file's icon, then the text, whose one word is `text`. */
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

test('an image loads from the web, as a data URL, or from the Host; anything else shows its alt text', () => {
  const html = renderMarkdown(
    '![chart](out/chart.png) and ![logo](https://example.com/l.png) and ![dot](data:image/png;base64,AAAA) and ![mail](mailto:a@b.c)',
    { files, hostImages: shownImages('/work/out/chart.png') },
  )
  // A click on any of them shows it large in the media viewer, so none is wrapped in a link.
  expect(html).toContain(`<img src="/raw?path=${encodeURIComponent('/work/out/chart.png')}" alt="chart" data-host-file="/work/out/chart.png" />`)
  expect(html).toContain('<img src="https://example.com/l.png" alt="logo" />')
  expect(html).not.toContain('<a ')
  // Each stands on its own line, the words between them on theirs.
  expect(html).toContain('<p>and</p>\n<p class="media-lone"><img src="data:image/png;base64,AAAA" alt="dot" /></p>\n<p>and mail</p>')
})

test('a link or image path starting ~/ names the Host home, and stays text while the home is not known', () => {
  const markdown = '[todo](~/notes/todo.md) ![chart](~/out/chart.png)'
  const html = renderMarkdown(markdown, { files: { ...files, home: '/Users/alex' }, hostImages: shownImages('/Users/alex/out/chart.png') })
  expect(html).toContain(fileLink('/Users/alex/notes/todo.md', 'todo'))
  expect(html).toContain(`<img src="/raw?path=${encodeURIComponent('/Users/alex/out/chart.png')}" alt="chart" data-host-file="/Users/alex/out/chart.png" />`)
  expect(renderMarkdown(markdown, { files })).toBe('<p>todo chart</p>\n')
})

test('an image inside a link follows the link', () => {
  expect(renderMarkdown('[![build](https://example.com/badge.svg)](https://example.com/ci)', { files }))
    .toBe('<p class="media-lone"><a href="https://example.com/ci" target="_blank" rel="noopener noreferrer nofollow"><img src="https://example.com/badge.svg" alt="build" /></a></p>\n')
  expect(renderMarkdown('[![chart](out/chart.png)](docs/charts.md)', { files, hostImages: shownImages('/work/out/chart.png') }))
    .toContain(`<a href="/work/docs/charts.md" data-file-link><img src="/raw?path=${encodeURIComponent('/work/out/chart.png')}" alt="chart" data-host-file="/work/out/chart.png" /></a>`)
})

test('a Host image shows its alt text in the placeholder box while its file is read and when the Host has no file there', () => {
  const box = '<span class="image-placeholder" role="img" aria-label="chart">chart</span>'
  const reading = renderMarkdown('![chart](out/chart.png)', { files, hostImages: new Map() })
  expect(reading).toBe(`<p class="media-lone">${box}</p>\n`)
  const none = renderMarkdown('see ![chart](out/chart.png)', { files, hostImages: new Map([['/work/out/chart.png', { state: 'none' }]]) })
  expect(none).toBe(`<p>see ${box}</p>\n`)
})

test('without a Host, paths stay text and Host images show their alt text', () => {
  const html = renderMarkdown('[plot](scripts/plot.py) ![chart](out/chart.png) ![logo](https://example.com/l.png)')
  expect(html).not.toContain('data-file-link')
  // A Host image's alt text is words; the web image stands on its own line.
  expect(html).toBe('<p>plot chart</p>\n<p class="media-lone"><img src="https://example.com/l.png" alt="logo" /></p>\n')
})

test('emphasis beside CJK text formats as a CJK writer means it', () => {
  // Plain CommonMark reads these stars and tildes, beside CJK punctuation, as text (`composer.md` § Writing a message).
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

test('inline code that names a file the Host has opens it as a link of words; any other code, and plain text, stay as they are', () => {
  const reply = 'See `src/a.ts:42`, `src/gone.ts`, `README.md`, `runner` and src/b.ts.'
  // Until the Host answers, all of it is code.
  expect(renderMarkdown(reply, { files, codeFiles: null })).not.toContain('data-file-link')
  const html = renderMarkdown(reply, { files, codeFiles: new Set(['/work/src/a.ts', '/work/README.md', '/work/src/b.ts']) })
  // A link followed by ASCII punctuation keeps no space before it, as any icon link.
  const a = `<a href="/work/src/a.ts" data-file-link data-line="42" class="icon-link icon-link-tight">${lead('data-file-icon="a.ts"', 'src')}/a.ts:42</a>`
  expect(html).toBe(`<p>See ${a}, <code>src/gone.ts</code>, ${fileLink('/work/README.md', 'README.md').replace('"icon-link"', '"icon-link icon-link-tight"')}, <code>runner</code> and src/b.ts.</p>\n`)
})

test('a link to an agent shows its mark and opens it; a number the conversation does not have is its text', () => {
  const agents: MessageAgents = {
    find: (number) => (number === 4 ? { number: 4, description: 'File icons', run: null } : null),
    open: () => {},
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
  }
  const classes = (markdown: string) => [...renderMarkdown(markdown, { files, agents }).matchAll(/class="((?:agent-link )?icon-link[^"]*)"/g)].map((match) => match[1])
  // "cwd check;" reads as written, not "cwd check ;".
  expect(classes('Ask [cwd check](agent:4); then [the notes](notes.md), and [more](agent:5).')).toEqual([
    'agent-link icon-link icon-link-tight', 'icon-link icon-link-tight', 'agent-link icon-link icon-link-tight',
  ])
  // A word after the link, and full-width punctuation, which carries its own space, keep the link's.
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

// The attachments the agent uploaded (`commands.md` § Attachment commands):
// `attachment:a3` names one of the conversation's, whatever its file became.

const attachments: Record<string, AttachmentLookup> = {
  a1: { state: 'found', attachment: { name: 'login.png', mediaType: 'image/png', sha256: '1'.repeat(64), url: '/blobs/1?type=image%2Fpng', width: 480, height: 300 } },
  a2: { state: 'found', attachment: { name: 'flow.mp4', mediaType: 'video/mp4', sha256: '2'.repeat(64), url: '/blobs/2?type=video%2Fmp4', width: 1280, height: 720 } },
  a3: { state: 'found', attachment: { name: 'report.pdf', mediaType: 'application/pdf', sha256: '3'.repeat(64), url: '/blobs/3?type=application%2Fpdf' } },
  a4: { state: 'loading' },
  a5: { state: 'failed' },
  a6: { state: 'found', attachment: { name: 'clip.webm', mediaType: 'video/webm', sha256: '6'.repeat(64), url: '/blobs/6?type=video%2Fwebm' } },
  a7: { state: 'found', attachment: { name: 'frame.png', mediaType: 'image/png', sha256: '7'.repeat(64), url: '/blobs/7?type=image%2Fpng', width: 171, height: 40 } },
  a45: { state: 'found', attachment: { name: 'words.png', mediaType: 'image/png', sha256: '45'.repeat(32), url: '/blobs/45?type=image%2Fpng', width: 2560, height: 1720 } },
  a46: { state: 'found', attachment: { name: 'words-2.png', mediaType: 'image/png', sha256: '46'.repeat(32), url: '/blobs/46?type=image%2Fpng', width: 1280, height: 720 } },
  a48: { state: 'found', attachment: { name: 'text.png', mediaType: 'image/png', sha256: '48'.repeat(32), url: '/blobs/48?type=image%2Fpng', width: 2560, height: 1720 } },
}
const withAttachments: MessageFiles = {
  ...files,
  attachment: (id) => attachments[id] ?? { state: 'missing' },
}

/** The stored file an attachment's link carries for a drag into a composer. */
function file(id: string): string {
  const lookup = attachments[id]
  if (lookup?.state !== 'found')
    throw new Error(`No attachment ${id}`)
  const { name, sha256, mediaType } = lookup.attachment
  return ` data-attachment-file="${JSON.stringify({ type: 'stored', name, sha256, mediaType }).replaceAll('"', '&quot;')}"`
}

test('an attachment image shows, and a click on it shows it large', () => {
  expect(renderMarkdown('![The fixed sign-in page](attachment:a1)', { files: withAttachments }))
    .toBe(`<p class="media-lone"><a href="/blobs/1?type=image%2Fpng" data-attachment-image="login.png"${file('a1')} data-keeps-windows><img src="/blobs/1?type=image%2Fpng" alt="The fixed sign-in page" data-width="480" data-height="300" /></a></p>\n`)
})

const mark = '<span class="media-play-mark" aria-hidden="true"></span>'

test('an attachment video shows its first frame with a play mark, with the size its record says, and a click plays it large', () => {
  expect(renderMarkdown('![The flow](attachment:a2)', { files: withAttachments }))
    .toBe(`<p class="media-lone"><a href="/blobs/2?type=video%2Fmp4" data-attachment-video="flow.mp4"${file('a2')} data-keeps-windows class="message-video"><video src="/blobs/2?type=video%2Fmp4" muted playsinline preload="metadata" aria-label="The flow" data-width="1280" data-height="720"></video>${mark}</a></p>\n`)
  // A record without a size leaves it to the first frame.
  expect(renderMarkdown('![The clip](attachment:a6)', { files: withAttachments }))
    .toBe(`<p class="media-lone"><a href="/blobs/6?type=video%2Fwebm" data-attachment-video="clip.webm"${file('a6')} data-keeps-windows class="message-video"><video src="/blobs/6?type=video%2Fwebm" muted playsinline preload="metadata" aria-label="The clip"></video>${mark}</a></p>\n`)
})

test('a link opens an attachment, with its file\'s icon: an image or a video large, any other file as a download', () => {
  const html = renderMarkdown('[shot](attachment:a1) [flow](attachment:a2) [report](attachment:a3) ![report](attachment:a3)', { files: withAttachments })
  const icon = (name: string, word: string) => `class="icon-link">${lead(`data-file-icon="${name}"`, word)}</a>`
  expect(html).toContain(`<a href="/blobs/1?type=image%2Fpng" data-attachment-image="login.png"${file('a1')} data-keeps-windows ${icon('login.png', 'shot')}`)
  expect(html).toContain(`<a href="/blobs/2?type=video%2Fmp4" data-attachment-video="flow.mp4"${file('a2')} data-keeps-windows ${icon('flow.mp4', 'flow')}`)
  const report = `<a href="/blobs/3?type=application%2Fpdf" download="report.pdf"${file('a3')} ${icon('report.pdf', 'report')}`
  expect(html).toContain(`${report} ${report}`)
})

test('an attachment number the conversation does not have says so beside its text', () => {
  const html = renderMarkdown('![The page](attachment:a9) [notes](attachment:a9)', { files: withAttachments })
  expect(html).toBe('<p>The page <span class="attachment-missing">No attachment a9 in this conversation</span> notes <span class="attachment-missing">No attachment a9 in this conversation</span></p>\n')
})

test('an attachment shows its text while the page asks for it, when asking failed, and outside a conversation', () => {
  expect(renderMarkdown('![shot](attachment:a4) [flow](attachment:a5)', { files: withAttachments }))
    .toBe('<p class="media-lone">shot</p>\n<p>flow</p>\n')
  expect(renderMarkdown('![shot](attachment:a1)', { files })).toBe('<p>shot</p>\n')
  expect(renderMarkdown('![shot](attachment:a1)')).toBe('<p>shot</p>\n')
})

// Images that follow each other stand side by side (`file-previews.md`
// § Files named in messages): a run is one row, each image an item of it.

const shot = (n: number) => `![shot ${n}](https://example.com/${n}.png)`
const item = (n: number) => `<img src="https://example.com/${n}.png" alt="shot ${n}" />`

test('images with only white space between them in one paragraph are one run', () => {
  expect(renderMarkdown(`${shot(1)} ${shot(2)}\n${shot(3)}`, { files }))
    .toBe(`<p class="media-run">${item(1)}${item(2)}${item(3)}</p>\n`)
})

test('paragraphs of images that follow each other are one run, a video among them', () => {
  expect(renderMarkdown(`${shot(1)}\n\n![The flow](attachment:a2)\n\n\n${shot(2)}`, { files: withAttachments }))
    .toBe(`<p class="media-run">${item(1)}<a href="/blobs/2?type=video%2Fmp4" data-attachment-video="flow.mp4"${file('a2')} data-keeps-windows class="message-video"><video src="/blobs/2?type=video%2Fmp4" muted playsinline preload="metadata" aria-label="The flow" data-width="1280" data-height="720"></video>${mark}</a>${item(2)}</p>\n`)
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
  expect(renderMarkdown('- **Text** ... (this one is 17 px).\n![shot](attachment:a48)\n- Next', { files: withAttachments }))
    .toBe(`<ul>\n<li><strong>Text</strong> ... (this one is 17 px).<p class="media-lone"><a href="/blobs/48?type=image%2Fpng" data-attachment-image="text.png"${file('a48')} data-keeps-windows><img src="/blobs/48?type=image%2Fpng" alt="shot" data-width="2560" data-height="1720" /></a></p>\n</li>\n<li>Next</li>\n</ul>\n`)
})

// The reply the user saw: a list item's words, then two screenshots one per
// line, showed each at full width under the other instead of side by side.
test('images after a list item’s words stand side by side under them', () => {
  const html = renderMarkdown('- **Words** ... text.\n  ![a](attachment:a45)\n  ![b](attachment:a46)', { files: withAttachments })
  expect(html).toStartWith('<ul>\n<li><strong>Words</strong> ... text.<p class="media-run"><a href="/blobs/45?type=image%2Fpng"')
  expect(html.match(/<img /g)).toHaveLength(2)
  expect(html).toContain('alt="b" data-width="1280" data-height="720" /></a></p>\n</li>')
})

test('an image inside a link counts as an image of a run, and follows its link', () => {
  expect(renderMarkdown(`[![build](https://example.com/badge.svg)](https://example.com/ci)\n${shot(1)}`, { files }))
    .toBe(`<p class="media-run"><a href="https://example.com/ci" target="_blank" rel="noopener noreferrer nofollow"><img src="https://example.com/badge.svg" alt="build" /></a>${item(1)}</p>\n`)
})

// Dollar signs follow Pandoc's rule (`file-previews.md` § Markdown); a file
// renders math through the same syntax.

test('prices stay text', () => {
  // A closing `$` needs no space before it and no digit after it.
  for (const text of ['Notion is $10 vs $12; Slack $7.25 vs $8.75', 'Plans cost $5/$6 a month'])
    expect(renderMarkdown(text)).toBe(`<p>${text}</p>\n`)
})

test('math renders on its own, beside Chinese text, and as a display', () => {
  expect(renderMarkdown('$x$')).toStartWith('<p><span class="katex">')
  const beside = renderMarkdown('面积为$x^2$平方米')
  expect(beside).toStartWith('<p>面积为<span class="katex">')
  expect(beside).toEndWith('</span>平方米</p>\n')
  expect(renderMarkdown('$$\\sum_i x_i$$')).toContain('<span class="katex-display">')
})

// The pictures a message shows that open in the media viewer, in order.

test('a message\'s pictures are its images that load and its attachments that are images or videos, in order', () => {
  const markdown = [
    '![chart](out/chart.png) and ![logo](https://example.com/l.png)',
    '![The flow](attachment:a2)',
    '![mail](mailto:a@b.c) ![report](attachment:a3) ![soon](attachment:a4) ![gone](attachment:a9)',
    '`![code](not-an-image.png)`',
    '![](attachment:a1)',
  ].join('\n\n')
  expect(messageMedia(markdown, withAttachments)).toEqual([
    { kind: 'image', file: { path: '/work/out/chart.png', contents }, name: 'chart' },
    { kind: 'image', src: 'https://example.com/l.png', name: 'logo' },
    { kind: 'video', src: '/blobs/2?type=video%2Fmp4', name: 'flow.mp4', size: { width: 1280, height: 720 } },
    { kind: 'image', src: '/blobs/1?type=image%2Fpng', name: 'login.png', size: { width: 480, height: 300 } },
  ])
})

test('an image inside a link follows the link and is not among a message\'s pictures', () => {
  expect(messageMedia('[![build](https://example.com/badge.svg)](https://example.com/ci) ![chart](out/chart.png)', files))
    .toEqual([{ kind: 'image', file: { path: '/work/out/chart.png', contents }, name: 'chart' }])
})
