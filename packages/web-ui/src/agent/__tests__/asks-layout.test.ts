import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// Side questions' cards in a reply. Two questions about words of one paragraph
// each set their card after it, in the order they were asked, and the layer
// comes to rest, moving nothing once they stand there. A Vue app on a happy-dom
// page; a few ms besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, ref } = await import('vue')
const { default: ReplyComments } = await import('../comments/ReplyComments.vue')
const { EMPTY_TRANSCRIPT } = await import('@demicodes/conversation-client')
type ShownSideQuestion = import('../comments/asks').ShownSideQuestion

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

const frame = () => new Promise<void>((done) => requestAnimationFrame(() => done()))

const REPLY = 'Uploads fail because the client gives up first. Upload in chunks of 4 MB.'

function question(id: string, quote: string, askedAt: string): ShownSideQuestion {
  const start = REPLY.indexOf(quote)
  return {
    id, blockId: 'text-1', start, end: start + quote.length, quote, question: `Why ${quote}?`, title: null,
    folded: false, askedAt, history: EMPTY_TRANSCRIPT, failures: {}, working: false,
  }
}

test('two cards on one paragraph stand after it in the order asked, and the layer comes to rest', async () => {
  const host = document.createElement('div')
  document.body.append(host)
  const rows = ref<HTMLElement | null>(null)
  const questions = ref([
    question('first', 'the client gives up', '2026-10-18T07:00:00.000Z'),
    question('second', 'Upload in chunks', '2026-10-18T07:01:00.000Z'),
  ])
  const app = createApp(defineComponent({
    setup: () => () => h('div', { ref: rows, style: 'position: relative' }, [
      h('div', { 'data-reply-text': 'text-1' }, [h('div', { class: 'markdown-body' }, [h('p', REPLY)])]),
      h(ReplyComments, {
        root: rows.value,
        sent: [],
        commenting: { waiting: [], add: () => {}, edit: () => {}, remove: () => {} },
        asking: {
          get questions() {
            return questions.value
          },
          ask: () => {},
          followUp: () => {},
          retry: () => {},
          fold: (id: string, folded: boolean) => {
            questions.value = questions.value.map((each) => (each.id === id ? { ...each, folded } : each))
          },
          read: () => {},
        },
      }),
    ]),
  }))
  app.mount(host)
  unmount = () => app.unmount()
  const settle = async () => {
    for (let count = 0; count < 6; count++) {
      await nextTick()
      await frame()
    }
  }
  await settle()
  const body = host.querySelector('.markdown-body')!
  const order = () => [...body.children].map((child) => (child as HTMLElement).dataset.askHost === undefined
    ? child.tagName
    : child.querySelector('[data-ask-card]')?.textContent?.includes('Why the client') ? 'first' : 'second')
  expect(order()).toEqual(['P', 'first', 'second'])

  // At rest, measuring again moves nothing.
  let moves = 0
  const watching = new MutationObserver((records) => (moves += records.length))
  watching.observe(body, { childList: true })
  await settle()
  watching.disconnect()
  expect(moves).toBe(0)

  // Folded and opened again, the first takes its place before the second.
  questions.value = questions.value.map((each) => (each.id === 'first' ? { ...each, folded: true } : each))
  await settle()
  expect(order()).toEqual(['P', 'second'])
  questions.value = questions.value.map((each) => (each.id === 'first' ? { ...each, folded: false } : each))
  await settle()
  expect(order()).toEqual(['P', 'first', 'second'])
})

// While the side question's turn is made, the card shows its status and no
// Reply; Reply comes back with the answer.
test('a card offers Reply only once its answer is made', async () => {
  const { default: AskCard } = await import('../comments/AskCard.vue')
  const { provideMediaViewer } = await import('../../files/media-viewer')
  const host = document.createElement('div')
  document.body.append(host)
  const turns = ref([{ question: 'Why stale?', status: 'Thinking…' as string | null, answer: '', streaming: false }])
  const app = createApp(defineComponent({
    setup: () => {
      provideMediaViewer()
      return () => h(AskCard, { node: 'side', title: null, turns: turns.value })
    },
  }))
  app.mount(host)
  unmount = () => app.unmount()
  await nextTick()
  const reply = () => [...host.querySelectorAll('[role="button"]')].some((button) => button.textContent?.trim() === 'Reply')
  expect(reply()).toBe(false)
  turns.value = [{ question: 'Why stale?', status: null, answer: 'Its TTL', streaming: true }]
  await nextTick()
  expect(reply()).toBe(false)
  turns.value = [{ question: 'Why stale?', status: null, answer: 'Its TTL is an hour.', streaming: false }]
  await nextTick()
  expect(reply()).toBe(true)
})

// A card's answer is the side question's agent's. Set in the root's reply, it
// shows the attachment the side question uploaded, and the reply's own as
// missing. Planted defect this catches: a card that asks as the reply it sits
// in, so the side question's file is missing and the root's shows. A few
// milliseconds.
test('a card shows what its side question uploaded, not what the reply it sits in did', async () => {
  const { default: AskCard } = await import('../comments/AskCard.vue')
  const { provideMediaViewer } = await import('../../files/media-viewer')
  const { provideMessageFiles } = await import('../../markdown/message-files')
  const uploaders = new Map([['a1', null], ['a3', 'side']])
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp(defineComponent({
    setup: () => {
      provideMediaViewer()
      // The root's reply, whose own agent's files are the conversation's
      // agent's.
      provideMessageFiles(() => ({
        cwd: '/work',
        attachment: (id: string, node: string | null) => uploaders.has(id) && uploaders.get(id) === node
          ? { state: 'found' as const, attachment: { name: `${id}.png`, mediaType: 'image/png', url: `/blobs/${id}`, size: 4, sha256: 'c'.repeat(64), width: 4, height: 3 } }
          : { state: 'missing' as const },
      }))
      const answer = 'Here:\n\n<attachment id="a3" />\n\nAnd:\n\n<attachment id="a1" />'
      return () => h(AskCard, { node: 'side', title: null, turns: [{ question: 'Show it', status: null, answer, streaming: false }] })
    },
  }))
  app.mount(host)
  unmount = () => app.unmount()
  await nextTick()
  expect([...host.querySelectorAll('img')].map((image) => image.getAttribute('alt'))).toEqual(['a3.png'])
  expect(host.textContent).toContain('No attachment a1 in this conversation')
})
