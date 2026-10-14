import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'
import { conversationSummary, productState } from '../__tests__/product-state'

// A reply that names `attachment:a2` before the agent uploads it shows it
// missing, and once the upload is stored, the conversation's summary on the
// product channel carries it as the latest attachment and the reply shows
// the image in place (`file-previews.md` § Files named in messages). The
// reply is the product's message view on a page of happy-dom, given the
// conversation's attachments as the conversation page gives them; the
// backend is a played channel and a stubbed fetch. A few milliseconds.

GlobalRegistrator.register({ url: 'http://127.0.0.1:3271/chat' })
// After the page, since the components ask for its elements and the played
// channel is one of its event targets.
const { playChannels } = await import('../__tests__/sync-channel')
await import('../__tests__/vue-components')
const { createApp, h } = await import('vue')
const { createPinia } = await import('pinia')
const { default: StreamedMarkdown } = await import('@demicodes/web-ui/ui/StreamedMarkdown.vue')
const { provideMessageFiles } = await import('@demicodes/web-ui/markdown/message-files')
const { provideMediaViewer } = await import('@demicodes/web-ui/files/media-viewer')
const { useProduct } = await import('../state/product')
const { useMessageAttachments } = await import('./attachments')

const CONVERSATION = '6a1d2c3b-4e5f-4a6b-8c7d-9e0f1a2b3c4d'
const BLOB = 'b'.repeat(64)
const realFetch = globalThis.fetch
const channels = playChannels()

afterEach(() => {
  globalThis.fetch = realFetch
  channels.restore()
})

/** Answers the attachment route from `stored`, by number, and keeps the numbers the page asked for. */
function stubBackend(stored: Set<string>): string[] {
  const asked: string[] = []
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const path = String(input)
    const prefix = `/api/conversations/${CONVERSATION}/attachments/`
    if (!path.startsWith(prefix))
      throw new Error(`Unexpected request: ${path}`)
    const id = path.slice(prefix.length)
    asked.push(id)
    return stored.has(id)
      ? Response.json({ id, name: 'concept-28.png', mediaType: 'image/png', size: 4, width: 4, height: 3, blob: BLOB })
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
      return () => h(StreamedMarkdown, { content: 'A (recommended)\n\n![Concept A](attachment:a2)' })
    },
  })
  app.use(pinia)
  // The reply names a2, which the conversation does not have yet.
  const rendered = changed(page, () => page.textContent?.includes('No attachment') === true)
  app.mount(page)
  await rendered
  const reply = page.firstElementChild!
  expect(reply.textContent).toContain('Concept A No attachment a2 in this conversation')
  expect(asked).toEqual([])

  // The upload stores a2, and the backend sends the conversation's summary.
  stored.add('a2')
  const shown = changed(reply, () => reply.querySelector('img') !== null)
  channels.last().send({ type: 'conversation', conversation: conversationSummary(CONVERSATION, 'Work', { lastAttachment: 2 }) })
  await shown

  // The same reply, not one mounted again, shows the image.
  expect(page.firstElementChild).toBe(reply)
  const image = reply.querySelector('img')!
  expect(image.getAttribute('alt')).toBe('Concept A')
  expect(image.getAttribute('src')).toContain(BLOB)
  expect(reply.textContent).not.toContain('No attachment')
  expect(asked).toEqual(['a2'])
  app.unmount()
  product.stop()
  // The deadline only guards against a reply that never shows it.
}, 2000)
