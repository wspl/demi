import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// Comments on a reply's words (`comments-and-ask.md`): Comment on selected
// words saves the comment with its quote and where it stands; the reply
// marks a waiting comment and, once a message sent it, marks it dimmer; the
// sent message's bubble shows each quote and its comment before its text.
// Vue apps on a happy-dom page; a few ms besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, ref } = await import('vue')
const { default: UserBlock } = await import('../blocks/UserBlock.vue')
const { default: ReplyComments } = await import('../comments/ReplyComments.vue')
const { provideBlobUrl } = await import('../media-source')
type MessageComment = import('@demicodes/protocol').MessageComment

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

const frame = () => new Promise<void>((done) => requestAnimationFrame(() => done()))

test('a message sent with comments shows each quoted text as a quote and its comment before its own text', async () => {
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp(defineComponent({
    setup: () => {
      provideBlobUrl((source) => `blob:${source}`)
      return () => h(UserBlock, {
      content: [{ type: 'text', text: 'Otherwise go ahead.' }],
      comments: [
        { id: 'c1', quote: 'Upload in chunks of 4 MB.', text: '4 MB feels large on 2G.' },
        { id: 'c2', quote: 'the same 30-second timeout', text: 'Should uploads get their own?' },
      ],
    })
    },
  }))
  app.mount(root)
  unmount = () => app.unmount()
  await nextTick()
  const quotes = [...root.querySelectorAll('blockquote')].map((quote) => quote.textContent?.trim())
  expect(quotes).toEqual(['Upload in chunks of 4 MB.', 'the same 30-second timeout'])
  const text = root.textContent!.replace(/\s+/g, ' ')
  const order = ['Upload in chunks of 4 MB.', '4 MB feels large on 2G.', 'the same 30-second timeout', 'Should uploads get their own?', 'Otherwise go ahead.']
    .map((part) => text.indexOf(part))
  expect(order.every((at, index) => at >= 0 && (index === 0 || at > order[index - 1]!))).toBe(true)
})

const REPLY = 'Uploads fail because the client gives up first. Upload in chunks of 4 MB.'
const START = REPLY.indexOf('Upload in chunks')

/** A reply's text block as the transcript draws it, with the comments layer over it. */
async function reply(options: { waiting: MessageComment[]; sent: MessageComment[] }) {
  const host = document.createElement('div')
  document.body.append(host)
  const waiting = ref(options.waiting)
  const sent = ref(options.sent)
  const added: MessageComment[] = []
  const rows = ref<HTMLElement | null>(null)
  const layer = ref<InstanceType<typeof ReplyComments>>()
  const app = createApp(defineComponent({
    setup: () => () => h('div', { ref: rows, style: 'position: relative' }, [
      h('div', { 'data-reply-text': 'text-1' }, [
        h('div', { class: 'markdown-body' }, [h('p', REPLY)]),
      ]),
      h(ReplyComments, {
        ref: layer,
        root: rows.value,
        sent: sent.value,
        commenting: {
          get waiting() {
            return waiting.value
          },
          add: (comment: MessageComment) => {
            added.push(comment)
            waiting.value = [...waiting.value, comment]
          },
          edit: () => {},
          remove: (id: string) => (waiting.value = waiting.value.filter((each) => each.id !== id)),
        },
      }),
    ]),
  }))
  app.mount(host)
  unmount = () => app.unmount()
  await nextTick()
  await frame()
  await nextTick()
  return { host, waiting, sent, added, layer, text: host.querySelector('p')!.firstChild as Text }
}

const chunks: MessageComment = { id: 'c1', blockId: 'text-1', start: START, end: START + 25, quote: 'Upload in chunks of 4 MB.', text: 'Smaller?' }

test('Comment on words selected in a reply saves them with where they stand, and waits while the reply is written', async () => {
  const page = await reply({ waiting: [], sent: [] })
  const range = document.createRange()
  range.setStart(page.text, START)
  range.setEnd(page.text, START + 25)
  window.getSelection()!.removeAllRanges()
  window.getSelection()!.addRange(range)
  const replyRoot = page.host.querySelector<HTMLElement>('[data-reply-text]')!
  let streaming = true
  const [tool] = page.layer.value!.selectionTools({ blockId: 'text-1', root: replyRoot }, () => streaming)
  expect([tool!.label, tool!.disabled, tool!.disabledReason]).toEqual(['Comment', true, 'Wait for this reply to finish.'])
  streaming = false
  expect(tool!.disabled).toBe(false)
  tool!.run!()
  await nextTick()
  await frame()
  await nextTick()
  const field = page.host.querySelector('textarea')!
  field.value = 'Smaller?'
  field.dispatchEvent(new Event('input'))
  await nextTick()
  field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
  await nextTick()
  expect(page.added.map(({ id: _id, ...rest }) => rest)).toEqual([{ blockId: 'text-1', start: START, end: START + 25, quote: 'Upload in chunks of 4 MB.', text: 'Smaller?' }])
})

test('a waiting comment marks its words, and once a message sent it the mark is dimmer', async () => {
  const page = await reply({ waiting: [chunks], sent: [] })
  const marker = () => page.host.querySelector('[data-comment-marker]')
  expect(marker()?.classList.contains('comment-marker-sent')).toBe(false)
  // The message is sent: the comment leaves the draft and the sent message holds it.
  page.waiting.value = []
  page.sent.value = [chunks]
  await nextTick()
  await frame()
  await nextTick()
  expect(marker()?.classList.contains('comment-marker-sent')).toBe(true)
  // A block that no longer holds the quoted words shows no mark.
  page.sent.value = [{ ...chunks, quote: 'Download in chunks of 4 MB.' }]
  await nextTick()
  await frame()
  await nextTick()
  expect(marker()).toBeNull()
})

/**
 * Mounts `render` with what a page provides, in a happy-dom page laid out
 * as the list needs: its viewport is tall and each of its rows short, so it
 * shows them all. Resolves once `ready` holds and the comments layer has had
 * its frames to draw.
 */
async function mounted(render: () => ReturnType<typeof h>, ready: (host: HTMLElement) => boolean): Promise<HTMLElement> {
  const { provideMediaViewer } = await import('../../files/media-viewer')
  const rowHigh = (el: Element) => (el.hasAttribute('data-index') ? 40 : 2000)
  const real = {
    rect: Element.prototype.getBoundingClientRect,
    client: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight'),
    offset: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetHeight'),
  }
  Element.prototype.getBoundingClientRect = function () {
    return DOMRect.fromRect({ x: 0, y: 0, width: 800, height: rowHigh(this) })
  }
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', { configurable: true, get() { return rowHigh(this) } })
  Object.defineProperty(HTMLElement.prototype, 'offsetHeight', { configurable: true, get() { return rowHigh(this) } })
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp(defineComponent({
    setup: () => {
      provideBlobUrl((source) => `blob:${source}`)
      provideMediaViewer()
      return render
    },
  }))
  app.mount(host)
  unmount = () => {
    app.unmount()
    Element.prototype.getBoundingClientRect = real.rect
    for (const [name, descriptor] of [['clientHeight', real.client], ['offsetHeight', real.offset]] as const) {
      if (descriptor) Object.defineProperty(HTMLElement.prototype, name, descriptor)
    }
  }
  for (let tries = 0; tries < 200 && !ready(host); tries += 1) {
    await frame()
    await nextTick()
  }
  for (let more = 0; more < 5; more += 1) {
    await frame()
    await nextTick()
  }
  return host
}

/** A conversation whose second message comments on the first reply, as a `user` block or as another kind. */
async function commentedHistory() {
  const { createdAt, model, userBlock } = await import('./agent-harness')
  return {
    createdAt,
    model,
    userBlock,
    before: [
      userBlock('u-1', 't-1', 'Why do uploads fail?'),
      { type: 'text' as const, id: 't-1', createdAt, model, text: REPLY },
    ],
    comment: { ...chunks, blockId: 't-1' },
  }
}

const quotesOf = (host: HTMLElement) =>
  [...host.querySelectorAll('[data-sent-comments] blockquote')].map((quote) => quote.textContent?.trim())

test('a shared page shows a message\'s quotes and comments, and no mark on the reply', async () => {
  const { default: SharedConversation } = await import('../../share/SharedConversation.vue')
  const { heldTranscript } = await import('@demicodes/conversation-client')
  const { before, comment, userBlock, createdAt, model } = await commentedHistory()
  const blocks = [
    ...before,
    { ...userBlock('u-2', 't-2', 'Otherwise go ahead.'), comments: [comment] },
    { type: 'text' as const, id: 't-2', createdAt, model, text: 'Starting at 1 MB.' },
  ]
  const host = await mounted(() => h(SharedConversation, {
    title: 'Slow uploads',
    history: heldTranscript(blocks.length, [{ start: 0, blocks }]),
    load: 'ready',
    attachment: () => ({ state: 'missing' as const }),
  }), (host) => quotesOf(host).length > 0 && host.textContent!.includes(REPLY))
  expect(quotesOf(host)).toEqual(['Upload in chunks of 4 MB.'])
  expect(host.textContent).toContain(REPLY)
  expect(host.querySelector('[data-comment-marker]')).toBeNull()
})

test('a steer carries its comments: its bubble shows each quote and comment, waiting and once the agent read it, and the reply\'s mark is dimmer', async () => {
  const { default: AgentMessageList } = await import('../AgentMessageList.vue')
  const { before, comment, model, createdAt } = await commentedHistory()
  const later = { ...comment, id: 'c2', start: 0, end: 7, quote: 'Uploads', text: 'Which ones?' }
  const content = [{ type: 'text' as const, text: 'Otherwise go ahead.' }]
  const props = {
    conversationId: 'c',
    load: 'ready' as const,
    phase: 'running' as const,
    atStart: true,
    atEnd: true,
    queue: [],
    persistedScrollState: undefined,
    bottomOffset: 0,
    commenting: { waiting: [], add: () => {}, edit: () => {}, remove: () => {} },
  }
  const written = { type: 'steer' as const, id: 's-1', turnId: 't-1', createdAt, model, content, comments: [comment] }
  const host = await mounted(() => h(AgentMessageList, {
    ...props,
    blocks: [...before, written],
    pendingSteers: [{ id: 's-2', content, comments: [later] }],
  }), (host) => quotesOf(host).length === 2 && host.querySelectorAll('[data-comment-marker]').length > 0)
  expect(quotesOf(host)).toEqual(['Upload in chunks of 4 MB.', 'Uploads'])
  const markers = [...host.querySelectorAll('[data-comment-marker]')]
  expect(markers.map((marker) => marker.getAttribute('aria-label'))).toEqual(['2 Comments'])
  expect(markers.every((marker) => marker.classList.contains('comment-marker-sent'))).toBe(true)
})

// Copy Message and Send to Conversation take a sent message's comments as the
// model reads them, quotes and comments before its text, a message of
// comments alone included (`comments-and-ask.md` § Sending them). Before,
// they took the text alone, and a message of comments alone copied nothing.
test.each([
  { holds: 'text', content: [{ type: 'text' as const, text: 'Otherwise go ahead.' }], text: 'Otherwise go ahead.' },
  { holds: 'no text', content: [], text: '' },
])('Copy Message and Send to Conversation on a message with comments and $holds take its quotes and comments before its text', async ({ content, text }) => {
  const menus = await import('../../ui/context-menu')
  const { provideSendTargets } = await import('../send-to-conversation')
  const { provideMediaViewer } = await import('../../files/media-viewer')
  const written: ClipboardItem[][] = []
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { write: async (items: ClipboardItem[]) => void written.push(items) } })
  const opened: import('../../ui/context-menu').ContextMenuOpening[] = []
  const uninstall = menus.installContextMenus({
    phone: () => false,
    services: { copy: () => {}, copyImage: () => {}, save: () => {}, openLink: () => {} },
    open: (opening) => {
      opened.push(opening)
      return true
    },
  })
  const sent: string[] = []
  const comments = [
    { id: 'c1', quote: 'Upload in chunks\nof 4 MB.', text: '4 MB feels large on 2G.' },
    { id: 'c2', quote: 'the same 30-second timeout', text: 'Should uploads get their own?' },
  ]
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp(defineComponent({
    setup: () => {
      provideBlobUrl((source) => `blob:${source}`)
      provideMediaViewer()
      provideSendTargets({ recent: () => [], search: async () => [], describe: () => null, shown: () => 'here', send: (_to, words) => void sent.push(words.text) })
      return () => h(UserBlock, { content, comments, editable: true })
    },
  }))
  app.mount(host)
  unmount = () => {
    app.unmount()
    uninstall()
  }
  await nextTick()
  const read = '> Upload in chunks\n> of 4 MB.\n\n4 MB feels large on 2G.\n\n> the same 30-second timeout\n\nShould uploads get their own?'
  const whole = text ? `${read}\n\n${text}` : read
  host.querySelector('[data-sent-comments]')!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10, button: 2 }))
  const rows = opened[0]!.menu.groups().flat()
  await rows.find((row) => row.label === 'Copy Message')!.run!()
  await new Promise((resolve) => setTimeout(resolve))
  expect(await (await written[0]![0]!.getType('text/plain')).text()).toBe(whole)
  // Send to Conversation, to a new conversation.
  const submenu = document.createElement('div')
  document.body.append(submenu)
  const menu = createApp({ render: () => rows.find((row) => row.label === 'Send to Conversation')!.submenu!() })
  menu.mount(submenu)
  await nextTick()
  submenu.querySelector<HTMLElement>('[role=menuitem]')!.click()
  menu.unmount()
  expect(sent).toEqual([whole])
})

// A comment's anchor is character offsets into the reply's own text
// (`comments-and-ask.md` § Sending them): a selection across code and a link
// finds the same words once the reply is drawn again with its text split
// otherwise, as a theme change re-coloring its code does, and a control set
// into the reply, as a code block's Copy, is not counted.
test('an anchor across code and a link finds the same words after the reply is drawn again', async () => {
  const { anchorOf, rangeOf } = await import('../comments/anchors')
  const draw = (code: string, control: string) => {
    const root = document.createElement('div')
    root.innerHTML = `<pre><button>${control}</button><code>${code}</code></pre>`
      + '<p>Call <code>fetch</code> with <a href="https://example.com">the retry docs</a> in mind.</p>'
    document.body.append(root)
    return root
  }
  const before = draw('let size = 4', 'Copy')
  const fetchText = before.querySelector('p code')!.firstChild as Text
  const linkText = before.querySelector('p a')!.firstChild as Text
  const range = document.createRange()
  range.setStart(fetchText, 2)
  range.setEnd(linkText, 9)
  const anchor = anchorOf(before, range)!
  expect(range.toString()).toBe('tch with the retry')
  // Drawn again: the code block's words in spans of their colors, its Copy just used.
  const after = draw('<span>let</span> <span>size</span> = <span>4</span>', 'Copied')
  expect(rangeOf(after, anchor)?.toString()).toBe('tch with the retry')
})
