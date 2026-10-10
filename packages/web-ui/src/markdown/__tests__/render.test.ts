import { expect, test } from 'bun:test'
import { holdUndecidedMedium, renderMarkdown } from '../render'
import type { AttachmentLookup, MessageFiles } from '../types'

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
    '![chart](out/chart.png) and ![logo](https://example.com/l.png) and ![dot](data:image/png;base64,AAAA) and ![mail](mailto:a@b.c)',
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

test('emphasis beside CJK text formats as a CJK writer means it', () => {
  // Plain CommonMark reads these stars and tildes, beside CJK punctuation, as text (`product.md` § Writing a message).
  expect(renderMarkdown('**建议：**一组')).toBe('<p><strong>建议：</strong>一组</p>\n')
  expect(renderMarkdown('这是**“扫光”**效果')).toBe('<p>这是<strong>“扫光”</strong>效果</p>\n')
  expect(renderMarkdown('**注意：**これは')).toBe('<p><strong>注意：</strong>これは</p>\n')
  expect(renderMarkdown('**스크립트(script)**는')).toBe('<p><strong>스크립트(script)</strong>는</p>\n')
  expect(renderMarkdown('*强调：*文字 ~~删除：~~文字')).toBe('<p><em>强调：</em>文字 <del>删除：</del>文字</p>\n')
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

// The attachments the agent uploaded (`commands.md` § Attachment commands):
// `attachment:a3` names one of the conversation's, whatever its file became.

const attachments: Record<string, AttachmentLookup> = {
  a1: { state: 'found', attachment: { name: 'login.png', mediaType: 'image/png', url: '/blobs/1?type=image%2Fpng', width: 480, height: 300 } },
  a2: { state: 'found', attachment: { name: 'flow.mp4', mediaType: 'video/mp4', url: '/blobs/2?type=video%2Fmp4', width: 1280, height: 720 } },
  a3: { state: 'found', attachment: { name: 'report.pdf', mediaType: 'application/pdf', url: '/blobs/3?type=application%2Fpdf' } },
  a4: { state: 'loading' },
  a5: { state: 'failed' },
  a6: { state: 'found', attachment: { name: 'clip.webm', mediaType: 'video/webm', url: '/blobs/6?type=video%2Fwebm' } },
}
const withAttachments: MessageFiles = {
  ...files,
  attachment: (id) => attachments[id] ?? { state: 'missing' },
}

test('an attachment image shows, and a click on it shows it large', () => {
  expect(renderMarkdown('![The fixed sign-in page](attachment:a1)', { files: withAttachments }))
    .toBe('<p><a href="/blobs/1?type=image%2Fpng" data-attachment-image="login.png"><img src="/blobs/1?type=image%2Fpng" alt="The fixed sign-in page" data-width="480" data-height="300" /></a></p>\n')
})

const mark = '<span class="media-play-mark" aria-hidden="true"></span>'

test('an attachment video shows its first frame with a play mark, as large as its record says, and a click plays it large', () => {
  expect(renderMarkdown('![The flow](attachment:a2)', { files: withAttachments }))
    .toBe(`<p><a href="/blobs/2?type=video%2Fmp4" data-attachment-video="flow.mp4" class="message-video" style="--media-ratio: ${1280 / 720}; --media-width: 1280px"><video src="/blobs/2?type=video%2Fmp4" muted playsinline preload="metadata" aria-label="The flow" data-width="1280" data-height="720"></video>${mark}</a></p>\n`)
  // A record without a size leaves the frame 16:9 until it arrives.
  expect(renderMarkdown('![The clip](attachment:a6)', { files: withAttachments }))
    .toBe(`<p><a href="/blobs/6?type=video%2Fwebm" data-attachment-video="clip.webm" class="message-video"><video src="/blobs/6?type=video%2Fwebm" muted playsinline preload="metadata" aria-label="The clip"></video>${mark}</a></p>\n`)
})

test('a link opens an attachment: an image or a video large, any other file as a download', () => {
  const html = renderMarkdown('[shot](attachment:a1) [flow](attachment:a2) [report](attachment:a3) ![report](attachment:a3)', { files: withAttachments })
  expect(html).toContain('<a href="/blobs/1?type=image%2Fpng" data-attachment-image="login.png">shot</a>')
  expect(html).toContain('<a href="/blobs/2?type=video%2Fmp4" data-attachment-video="flow.mp4">flow</a>')
  expect(html).toContain('<a href="/blobs/3?type=application%2Fpdf" download="report.pdf">report</a> <a href="/blobs/3?type=application%2Fpdf" download="report.pdf">report</a>')
})

test('an attachment number the conversation does not have says so beside its text', () => {
  const html = renderMarkdown('![The page](attachment:a9) [notes](attachment:a9)', { files: withAttachments })
  expect(html).toBe('<p>The page <span class="attachment-missing">No attachment a9 in this conversation</span> notes <span class="attachment-missing">No attachment a9 in this conversation</span></p>\n')
})

test('an attachment shows its text while the page asks for it, when asking failed, and outside a conversation', () => {
  expect(renderMarkdown('![shot](attachment:a4) [flow](attachment:a5)', { files: withAttachments }))
    .toBe('<p>shot flow</p>\n')
  expect(renderMarkdown('![shot](attachment:a1)', { files })).toBe('<p>shot</p>\n')
  expect(renderMarkdown('![shot](attachment:a1)')).toBe('<p>shot</p>\n')
})

// Images that follow each other stand side by side (`file-previews.md`
// § Files named in messages): a run is one row, each image an item of it.

const shot = (n: number) => `![shot ${n}](https://example.com/${n}.png)`
const item = (n: number) => `<span style="height: 80px"><a href="https://example.com/${n}.png" target="_blank" rel="noopener noreferrer"><img src="https://example.com/${n}.png" alt="shot ${n}" style="width: 64px; height: 80px" /></a></span>`

test('images with only white space between them in one paragraph are one run', () => {
  expect(renderMarkdown(`${shot(1)} ${shot(2)}\n${shot(3)}`, { files }))
    .toBe(`<p class="media-run">${item(1)}${item(2)}${item(3)}</p>\n`)
})

test('paragraphs of images that follow each other are one run, a video among them', () => {
  expect(renderMarkdown(`${shot(1)}\n\n![The flow](attachment:a2)\n\n\n${shot(2)}`, { files: withAttachments }))
    .toBe(`<p class="media-run">${item(1)}<span style="height: 80px"><a href="/blobs/2?type=video%2Fmp4" data-attachment-video="flow.mp4" class="message-video"><video src="/blobs/2?type=video%2Fmp4" muted playsinline preload="metadata" aria-label="The flow" data-width="1280" data-height="720" style="width: 142px; height: 80px"></video>${mark}</a></span>${item(2)}</p>\n`)
})

// Before a byte of it loads, a medium whose record carries its size takes the
// thumbnail box that size makes, so nothing in the row moves when it loads;
// one whose size is unknown takes the narrowest box, or 16:9 for a video.
test('in a run, an attachment takes its final thumbnail box from its record before it loads', () => {
  const html = renderMarkdown('![sign-in](attachment:a1) ![flow](attachment:a2) ![clip](attachment:a6)', { files: withAttachments })
  expect(html).toContain('alt="sign-in" data-width="480" data-height="300" style="width: 128px; height: 80px" />')
  expect(html).toContain('aria-label="flow" data-width="1280" data-height="720" style="width: 142px; height: 80px"></video>')
  expect(html).toContain('aria-label="clip" style="width: 142px; height: 80px"></video>')
})

test('text, a list or a heading between images ends the run', () => {
  const html = renderMarkdown(
    `${shot(1)}\n${shot(2)}\n\nThen:\n\n${shot(3)}\n${shot(4)}\n\n- one\n\n${shot(5)} and ${shot(6)}\n\n# Done\n\n${shot(7)}`,
    { files },
  )
  expect(html.match(/class="media-run"/g)).toHaveLength(2)
  expect(html).toContain(`<p class="media-run">${item(1)}${item(2)}</p>`)
  expect(html).toContain(`<p class="media-run">${item(3)}${item(4)}</p>`)
  expect(html).not.toContain(`<span style="height: 80px"><a href="https://example.com/5.png"`)
  expect(html).not.toContain(`<span style="height: 80px"><a href="https://example.com/7.png"`)
})

test('a lone image keeps its paragraph', () => {
  expect(renderMarkdown(`Here it is:\n\n${shot(1)}\n\nThat is all.`, { files }))
    .toContain('<p><a href="https://example.com/1.png" target="_blank" rel="noopener noreferrer"><img src="https://example.com/1.png" alt="shot 1" /></a></p>')
})

test('an image inside a link counts as an image of a run, and follows its link', () => {
  expect(renderMarkdown(`[![build](https://example.com/badge.svg)](https://example.com/ci)\n${shot(1)}`, { files }))
    .toBe(`<p class="media-run"><span style="height: 80px"><a href="https://example.com/ci" target="_blank" rel="noopener noreferrer"><img src="https://example.com/badge.svg" alt="build" style="width: 64px; height: 80px" /></a></span>${item(1)}</p>\n`)
})

test('while a message streams, a lone image at its end waits for what follows', () => {
  const intro = 'The pages:\n\n'
  // The first image could stand alone or start a run, so it waits.
  expect(holdUndecidedMedium(`${intro}${shot(1)}`)).toBe(intro)
  expect(holdUndecidedMedium(`${intro}${shot(1)}\n!`)).toBe(intro)
  expect(holdUndecidedMedium(`${intro}${shot(1)}\n[![b](x.png)](y`)).toBe(intro)
  // A second one makes a run, which shows; images that join it show at once.
  expect(holdUndecidedMedium(`${intro}${shot(1)}\n${shot(2)}`)).toBe(`${intro}${shot(1)}\n${shot(2)}`)
  // Text after it makes it a lone image, which shows.
  expect(holdUndecidedMedium(`${intro}${shot(1)}\n\nDone`)).toBe(`${intro}${shot(1)}\n\nDone`)
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
