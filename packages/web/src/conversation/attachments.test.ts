import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, test } from 'bun:test'
import { conversationSummary, productState } from '../__tests__/product-state'

// A reply that names `attachment:a2` before the agent uploads it shows it
// missing, and once the upload is stored, the conversation's summary on the
// product channel carries it as the latest attachment and the reply shows
// the image in place. The reply is the product's message view on a page of
// happy-dom, given the conversation's attachments as the conversation page
// gives them; the backend is a played channel and a stubbed fetch. A few
// milliseconds.

GlobalRegistrator.register({ url: 'http://127.0.0.1:3271/chat' })
// After the page, since the components ask for its elements and the played
// channel is one of its event targets.
const { playChannels } = await import('../__tests__/sync-channel')
await import('@demicodes/web-ui/__tests__/vue-components')
const { createApp, h } = await import('vue')
const { createPinia } = await import('pinia')
const { default: StreamedMarkdown } = await import('@demicodes/web-ui/ui/StreamedMarkdown.vue')
const { provideMessageFiles } = await import('@demicodes/web-ui/markdown/message-files')
const { provideMediaViewer } = await import('@demicodes/web-ui/files/media-viewer')
const { useProduct } = await import('../state/product')
const { attachmentOrigin, useMessageAttachments } = await import('./attachments')
const { default: MediaView } = await import('@demicodes/web-ui/files/MediaView.vue')
const { HostFiles } = await import('@demicodes/web-ui/files/file-cache')
const { keptContents } = await import('@demicodes/web-ui/files/kept-source')
const { SimulatedWatch } = await import('@demicodes/web-ui/files/memory-source')
const { WatchChannel } = await import('@demicodes/web-ui/files/watch-channel')
const { FileBrowserError } = await import('@demicodes/web-ui/files/types')

const CONVERSATION = '6a1d2c3b-4e5f-4a6b-8c7d-9e0f1a2b3c4d'
const BLOB = 'b'.repeat(64)
const realFetch = globalThis.fetch
// Each test plays the page's channels anew, which the last one's restore ended.
let channels = playChannels()

beforeEach(() => {
  channels = playChannels()
})

afterEach(() => {
  globalThis.fetch = realFetch
  channels.restore()
})

/**
 * Answers the attachment route from `stored`, by number, and keeps the numbers
 * the page asked for; each is `held`, a PNG unless the test says. An
 * attachment's file check (`…/attachments/:id/file`) answers whether its
 * source's path is in `onHost`, the files of the Host it came from, and is
 * kept in `checked`.
 */
function stubBackend(
  stored: Set<string>,
  held: { name: string; mediaType: string; source?: { host: string; path: string } } = { name: 'concept-28.png', mediaType: 'image/png' },
  onHost: Set<string> = new Set(),
  checked: string[] = [],
): string[] {
  const asked: string[] = []
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const { pathname: path } = new URL(String(input), 'http://127.0.0.1:3271')
    const prefix = `/api/conversations/${CONVERSATION}/attachments/`
    if (!path.startsWith(prefix))
      throw new Error(`Unexpected request: ${path}`)
    const checking = /^(a\d+)\/file$/.exec(path.slice(prefix.length))
    if (checking) {
      checked.push(checking[1]!)
      return stored.has(checking[1]!) && held.source && onHost.has(held.source.path)
        ? new Response(null, { status: 204 })
        : Response.json({ requestId: 'req_test000000000002', code: 'not_found', message: 'No file' }, { status: 404 })
    }
    const id = path.slice(prefix.length)
    asked.push(id)
    return stored.has(id)
      ? Response.json({ id, ...held, size: 4, ...(held.mediaType.startsWith('image/') ? { width: 4, height: 3 } : {}), blob: BLOB })
      : Response.json({ requestId: 'req_test000000000001', code: 'not_found', message: `No attachment ${id}` }, { status: 404 })
  }) as typeof fetch
  return asked
}

/** Resolves once `done` holds, checked at each change of `element`'s content. */
function changed(element: Element, done: () => boolean): Promise<void> {
  return new Promise((resolve) => {
    const observer = new MutationObserver(() => {
      if (done()) {
        observer.disconnect()
        resolve()
      }
    })
    observer.observe(element, { childList: true, subtree: true, characterData: true })
  })
}

test('a reply that named an attachment before its upload shows the image in place once the summary announces it', async () => {
  const stored = new Set<string>()
  const asked = stubBackend(stored)
  const pinia = createPinia()
  const product = useProduct(pinia)
  product.start()
  channels.last().connect(productState({ conversations: [conversationSummary(CONVERSATION, 'Work', { lastAttachment: 1 })] }))
  const page = document.createElement('div')
  document.body.append(page)
  const app = createApp({
    setup() {
      provideMediaViewer()
      const attachments = useMessageAttachments(() => CONVERSATION)
      provideMessageFiles(() => ({ cwd: '/work', ...(attachments.value ? { attachment: attachments.value } : {}) }))
      return () => h(StreamedMarkdown, { content: 'A (recommended)\n\n<attachment id="a2" />' })
    },
  })
  app.use(pinia)
  // The reply names a2, which the conversation does not have yet.
  const rendered = changed(page, () => page.textContent?.includes('No attachment') === true)
  app.mount(page)
  await rendered
  const reply = page.firstElementChild!
  expect(reply.textContent).toContain('No attachment a2 in this conversation')
  expect(asked).toEqual([])

  // The upload stores a2, and the backend sends the conversation's summary.
  stored.add('a2')
  const shown = changed(reply, () => reply.querySelector('img') !== null)
  channels.last().send({ type: 'conversation', conversation: conversationSummary(CONVERSATION, 'Work', { lastAttachment: 2 }) })
  await shown

  // The same reply, not one mounted again, shows the image.
  expect(page.firstElementChild).toBe(reply)
  const image = reply.querySelector('img')!
  expect(image.getAttribute('alt')).toBe('concept-28.png')
  expect(image.getAttribute('src')).toContain(BLOB)
  expect(reply.textContent).not.toContain('No attachment')
  expect(asked).toEqual(['a2'])
  app.unmount()
  product.stop()
  // The deadline only guards against a reply that never shows it.
}, 2000)

// A reply shows only what its own agent uploaded: a subagent's reply that names
// the root's a2, which it never uploaded, shows a2 missing, and its own a3
// shows. The backend stub answers each number only to the agent that uploaded
// it, as the route does with `?node=`. Planted defect this catches: a lookup
// that asks without the reply's agent, so a guessed number shows another
// agent's file. A few milliseconds.
test('a subagent\'s reply shows the attachment it uploaded and a number it did not upload as missing', async () => {
  const CHILD = 'node-child'
  const uploaders = new Map([['a2', CONVERSATION], ['a3', CHILD]])
  const asked: string[] = []
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const url = new URL(String(input), 'http://127.0.0.1:3271')
    const prefix = `/api/conversations/${CONVERSATION}/attachments/`
    if (!url.pathname.startsWith(prefix))
      throw new Error(`Unexpected request: ${url.pathname}`)
    const id = url.pathname.slice(prefix.length)
    asked.push(`${id} ${url.searchParams.get('node')}`)
    return uploaders.get(id) === url.searchParams.get('node')
      ? Response.json({ id, name: `${id}.png`, mediaType: 'image/png', size: 4, width: 4, height: 3, blob: BLOB })
      : Response.json({ requestId: 'req_test000000000001', code: 'not_found', message: `No attachment ${id}` }, { status: 404 })
  }) as typeof fetch
  const pinia = createPinia()
  const product = useProduct(pinia)
  product.start()
  channels.last().connect(productState({ conversations: [conversationSummary(CONVERSATION, 'Work', { lastAttachment: 3 })] }))
  const page = document.createElement('div')
  document.body.append(page)
  const app = createApp({
    setup() {
      provideMediaViewer()
      const attachments = useMessageAttachments(() => CONVERSATION)
      provideMessageFiles(() => ({ cwd: '/work', node: CHILD, ...(attachments.value ? { attachment: attachments.value } : {}) }))
      return () => h(StreamedMarkdown, { content: 'The screenshots:\n\n<attachment id="a2" />\n\nMine:\n\n<attachment id="a3" />' })
    },
  })
  app.use(pinia)
  const shown = changed(page, () => page.querySelector('img') !== null && page.textContent?.includes('No attachment') === true)
  app.mount(page)
  await shown
  const reply = page.firstElementChild!
  expect(reply.textContent).toContain('No attachment a2 in this conversation')
  expect([...reply.querySelectorAll('img')].map((image) => image.getAttribute('alt'))).toEqual(['a3.png'])
  expect(asked.sort()).toEqual([`a2 ${CHILD}`, `a3 ${CHILD}`])
  app.unmount()
  product.stop()
}, 2000)

// An audio attachment plays where the reply delivers it, as a voice message
// does in Messages: its block shows its player with the file's name and length,
// its button plays and pauses it, and its seek bar moves it. Planted defect
// this catches: an audio attachment shown as a card, as any other file is. A
// few milliseconds.
test('an audio attachment shows its inline player with its name and length, and plays and seeks there', async () => {
  stubBackend(new Set(['a3']), { name: 'welcome.mp3', mediaType: 'audio/mpeg' })
  const pinia = createPinia()
  const product = useProduct(pinia)
  product.start()
  channels.last().connect(productState({ conversations: [conversationSummary(CONVERSATION, 'Work', { lastAttachment: 3 })] }))
  const page = document.createElement('div')
  document.body.append(page)
  const app = createApp({
    setup() {
      provideMediaViewer()
      const attachments = useMessageAttachments(() => CONVERSATION)
      provideMessageFiles(() => ({ cwd: '/work', ...(attachments.value ? { attachment: attachments.value } : {}) }))
      return () => h(StreamedMarkdown, { content: 'The voice-over:\n\n<attachment id="a3" />' })
    },
  })
  app.use(pinia)
  const shown = changed(page, () => page.querySelector('[data-audio-player]') !== null)
  app.mount(page)
  await shown
  const players = [...page.querySelectorAll<HTMLElement>('[data-audio-player]')]
  expect(page.querySelector('[data-attachment-card-body]')).toBeNull()
  const [player] = players
  const audio = player!.querySelector('audio')!
  expect(audio.getAttribute('src')).toContain(BLOB)
  expect(player!.textContent).toContain('welcome.mp3')
  const time = () => player!.querySelector('[data-audio-time]')!.textContent
  expect(time()).toBe('–:––')

  // Its header read, it says its length.
  Object.defineProperty(audio, 'duration', { value: 4.2, configurable: true })
  const measured = changed(player!, () => time() === '0:04')
  audio.dispatchEvent(new Event('loadedmetadata'))
  await measured

  // Its button plays it, then pauses it.
  let paused = true
  Object.defineProperty(audio, 'paused', { get: () => paused, configurable: true })
  audio.play = async () => {
    paused = false
    audio.dispatchEvent(new Event('play'))
  }
  audio.pause = () => {
    paused = true
    audio.dispatchEvent(new Event('pause'))
  }
  const button = player!.querySelector('button')!
  expect(button.getAttribute('aria-label')).toBe('Play welcome.mp3')
  const playing = changed(player!, () => button.getAttribute('aria-label') === 'Pause welcome.mp3')
  button.click()
  await playing
  button.click()
  expect(paused).toBe(true)

  // Its seek bar moves it, and the time shows where it is.
  const seek = player!.querySelector<HTMLInputElement>('input[type="range"]')!
  expect(seek.getAttribute('aria-label')).toBe('Position in welcome.mp3')
  const moved = changed(player!, () => time() === '0:02')
  seek.value = '2.5'
  seek.dispatchEvent(new Event('input'))
  await moved
  expect(audio.currentTime).toBe(2.5)
  app.unmount()
  product.stop()
}, 2000)

// Any other file the reply delivers is a card: a click on it opens the file in
// the work panel and never downloads it, and only its Download downloads, under
// the file's name. Planted defects this catches: a card that downloads on a
// click, as an attachment's link once did, and a Download that also opens the
// panel. A few milliseconds.
test('a file\'s card opens it in the work panel on a click, and only its Download downloads it', async () => {
  stubBackend(new Set(['a4']), { name: 'report.pdf', mediaType: 'application/pdf' })
  const pinia = createPinia()
  const product = useProduct(pinia)
  product.start()
  channels.last().connect(productState({ conversations: [conversationSummary(CONVERSATION, 'Work', { lastAttachment: 4 })] }))
  const opened: [string, string, number][] = []
  const downloads: [string, string][] = []
  const realClick = HTMLAnchorElement.prototype.click
  HTMLAnchorElement.prototype.click = function (this: HTMLAnchorElement) {
    downloads.push([this.getAttribute('href') ?? '', this.download])
  }
  const page = document.createElement('div')
  document.body.append(page)
  const app = createApp({
    setup() {
      provideMediaViewer()
      const attachments = useMessageAttachments(() => CONVERSATION)
      provideMessageFiles(() => ({
        cwd: '/work',
        ...(attachments.value ? { attachment: attachments.value } : {}),
        openAttachment: (id, attachment) => void opened.push([id, attachment.name, attachment.size]),
      }))
      return () => h(StreamedMarkdown, { content: 'The report:\n\n<attachment id="a4" />' })
    },
  })
  app.use(pinia)
  const shown = changed(page, () => page.querySelector('[data-attachment-card-body][role="button"]') !== null)
  app.mount(page)
  await shown
  try {
    const card = page.querySelector<HTMLElement>('[data-attachment-card-body]')!
    expect(card.textContent).toContain('report.pdf')
    expect(card.textContent).toContain('PDF Document · 4 B')

    card.click()
    expect(opened).toEqual([['a4', 'report.pdf', 4]])
    expect(downloads).toEqual([])

    card.querySelector<HTMLElement>('[role="button"][aria-label="Download"]')!.click()
    expect(downloads).toEqual([[expect.stringContaining(BLOB), 'report.pdf']])
    expect(opened).toHaveLength(1)
  } finally {
    HTMLAnchorElement.prototype.click = realClick
    app.unmount()
    product.stop()
  }
}, 2000)

/** Lets the page's answers, the backend's included, land: they land in tasks of their own. */
async function settle(): Promise<void> {
  for (let turn = 0; turn < 4; turn++)
    await new Promise((resolve) => setTimeout(resolve, 0))
}

/**
 * The conversation's Host as an attachment's opening asks it: device `dev-1`,
 * online, whose files hold `present`, which the backend's file check and the
 * Host's own contents read. No change of `present` is announced.
 */
function openingHost(present: Set<string>) {
  const kept = new HostFiles()
  const watched = new SimulatedWatch()
  const contents = keptContents({
    url: (path) => `/raw?path=${path}`,
    async describe(path) {
      if (!present.has(path))
        throw new FileBrowserError('not-found')
      return { value: { size: 9, modifiedAt: null, version: null }, readAt: watched.readAt() }
    },
  }, { files: kept, watch: new WatchChannel(kept, watched.open) })
  const host = { conversation: CONVERSATION, device: 'dev-1' as string | null, online: true }
  return { host, contents }
}

// An attachment the agent uploaded opens the file it was uploaded from, as it
// is now, while the conversation's Host is that device, online, and has the
// file; otherwise its copy. Each opening asks the backend once, with no answer
// kept from the last, and never while the Host is known offline, a stopped
// Cloud included, or is another Host. Planted defects this catches: a kept
// answer, which opens a file deleted since the last opening; asking an offline
// Host; and opening another Host's file. Its numbers are its own: the page
// keeps each attachment's record for its lifetime. A few milliseconds.
test('an attachment\'s card opens its file while the conversation\'s Host has it, and its copy otherwise', async () => {
  const present = new Set(['/work/report.pdf'])
  const checked: string[] = []
  stubBackend(new Set(['a14']), { name: 'report.pdf', mediaType: 'application/pdf', source: { host: 'dev-1', path: '/work/report.pdf' } }, present, checked)
  const pinia = createPinia()
  const product = useProduct(pinia)
  product.start()
  channels.last().connect(productState({ conversations: [conversationSummary(CONVERSATION, 'Work', { lastAttachment: 14 })] }))
  const { host, contents } = openingHost(present)
  const files: string[] = []
  const copies: string[] = []
  const page = document.createElement('div')
  document.body.append(page)
  const app = createApp({
    setup() {
      provideMediaViewer()
      const attachments = useMessageAttachments(() => CONVERSATION)
      provideMessageFiles(() => ({
        cwd: '/work',
        contents,
        ...(attachments.value ? { attachment: attachments.value } : {}),
        open: ({ path }) => void files.push(path),
        openAttachment: (id) => void copies.push(id),
        attachmentOrigin: (id, attachment) => attachmentOrigin(id, attachment, host),
      }))
      return () => h(StreamedMarkdown, { content: 'The report:\n\n<attachment id="a14" />' })
    },
  })
  app.use(pinia)
  const shown = changed(page, () => page.querySelector('[data-attachment-card-body][role="button"]') !== null)
  app.mount(page)
  await shown
  const open = async () => {
    page.querySelector<HTMLElement>('[data-attachment-card-body]')!.click()
    await settle()
  }
  try {
    await open()
    expect([files, copies, checked]).toEqual([['/work/report.pdf'], [], ['a14']])

    // Deleted with no notice of it: the next opening asks again.
    present.delete('/work/report.pdf')
    await open()
    expect([files.length, copies, checked.length]).toEqual([1, ['a14'], 2])

    host.online = false
    present.add('/work/report.pdf')
    await open()
    expect([files.length, copies.length, checked.length]).toEqual([1, 2, 2])

    host.online = true
    host.device = 'dev-2'
    await open()
    expect([files.length, copies.length, checked.length]).toEqual([1, 3, 2])
  } finally {
    app.unmount()
    product.stop()
  }
}, 2000)

// A picture the agent uploaded opens in the work panel's Media tab beside the
// reply: the file it was uploaded from while the Host has it, and its copy
// once the Host is offline, which is then not asked. Planted defects this
// catches: the tab showing the copy of a file the Host has, and the window
// opening over the tab. A few milliseconds.
test('a picture attachment opens in the Media tab, its file while the Host has it and its copy otherwise', async () => {
  const present = new Set(['/work/chart.png'])
  const checked: string[] = []
  stubBackend(new Set(['a15']), { name: 'chart.png', mediaType: 'image/png', source: { host: 'dev-1', path: '/work/chart.png' } }, present, checked)
  const pinia = createPinia()
  const product = useProduct(pinia)
  product.start()
  channels.last().connect(productState({ conversations: [conversationSummary(CONVERSATION, 'Work', { lastAttachment: 15 })] }))
  const { host, contents } = openingHost(present)
  let revealed = 0
  const page = document.createElement('div')
  document.body.append(page)
  const app = createApp({
    setup() {
      provideMediaViewer({
        reveal: () => {
          revealed += 1
          return true
        },
      })
      const attachments = useMessageAttachments(() => CONVERSATION)
      provideMessageFiles(() => ({
        cwd: '/work',
        contents,
        ...(attachments.value ? { attachment: attachments.value } : {}),
        attachmentOrigin: (id, attachment) => attachmentOrigin(id, attachment, host),
      }))
      return () => [
        h(StreamedMarkdown, { content: 'The chart:\n\n<attachment id="a15" />' }),
        h(MediaView, { shown: true }),
      ]
    },
  })
  app.use(pinia)
  const shown = changed(page, () => page.querySelector('a[data-attachment-id="a15"]') !== null)
  app.mount(page)
  await shown
  const pictureIn = () => page.querySelector('[data-media-view] img')?.getAttribute('src')
  try {
    page.querySelector<HTMLElement>('a[data-attachment-id="a15"]')!.click()
    await settle()
    expect(revealed).toBe(1)
    expect(pictureIn()).toBe('/raw?path=/work/chart.png')
    expect(page.querySelector('[data-media-view]')!.textContent).toContain('chart.png')

    host.online = false
    page.querySelector<HTMLElement>('a[data-attachment-id="a15"]')!.click()
    await settle()
    expect(revealed).toBe(2)
    expect(pictureIn()).toContain(BLOB)
    expect(checked).toEqual(['a15'])
  } finally {
    app.unmount()
    product.stop()
  }
}, 2000)
