import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// The full window of a side question (`comments-and-ask.md` § Side
// questions): opened from the card's ↗, it takes the focus, and closed, by
// Escape too, it gives it back to ↗ as the page moves focus, so the browser
// draws no ring there (`focus-by-page.ts`). A Vue app on a happy-dom page;
// a few ms besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, ref } = await import('vue')
const { default: SideQuestionWindow } = await import('../comments/SideQuestionWindow.vue')
const { default: DockWindowArea } = await import('../DockWindowArea.vue')
const { provideBlobUrl } = await import('../media-source')
const { EMPTY_TRANSCRIPT, heldTranscript } = await import('@demicodes/conversation-client')
const { model } = await import('./agent-harness')
type ShownSideQuestion = import('../comments/asks').ShownSideQuestion

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

const question: ShownSideQuestion = {
  id: 'side', blockId: 'reply', start: 0, end: 5, quote: 'stale', question: 'Why stale?', title: null,
  folded: false, askedAt: '2026-10-18T07:00:00.000Z', history: EMPTY_TRANSCRIPT, failures: {}, working: false,
}

test('the full window takes the focus and gives it back to ↗ without a focus ring', async () => {
  const focused: { target: EventTarget; visible: boolean | undefined }[] = []
  const focus = HTMLElement.prototype.focus
  HTMLElement.prototype.focus = function (this: HTMLElement, options?: FocusOptions & { focusVisible?: boolean }) {
    focused.push({ target: this, visible: options?.focusVisible })
    focus.call(this, options)
  }
  try {
    const host = document.createElement('div')
    document.body.append(host)
    const opener = document.createElement('span')
    opener.tabIndex = 0
    document.body.append(opener)
    opener.focus()
    const shown = ref<ShownSideQuestion | null>(null)
    const app = createApp(defineComponent({
      setup: () => {
        provideBlobUrl((source) => `blob:${source}`)
        return () => h(DockWindowArea, { area: 600 }, () => [
        h(SideQuestionWindow, { question: shown.value, onClose: () => (shown.value = null), onRead: () => {} }),
      ])
      },
    }))
    app.mount(host)
    unmount = () => app.unmount()

    shown.value = question
    await nextTick()
    await nextTick()
    const region = document.querySelector('[data-region]')
    expect(region).not.toBeNull()
    expect(document.activeElement).toBe(region)

    document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await nextTick()
    await nextTick()
    expect(shown.value).toBeNull()
    expect(document.activeElement).toBe(opener)
    expect(focused.at(-1)).toEqual({ target: opener, visible: false })
  } finally {
    HTMLElement.prototype.focus = focus
  }
})

// A side question about a picture (`comments-and-ask.md` § Images): the
// window's first message, in the side's transcript, shows the picture as its
// quote with the question under it, as a comment's bubble does; a message
// with comments is no subagent's brief, which would draw it empty.
test('a picture side question\'s first message in its window shows the picture quoted above the question', async () => {
  const { default: AgentMessageVirtualBlock } = await import('../blocks/AgentMessageVirtualBlock.vue')
  const { provideTranscript } = await import('../edit-selection')
  const { storedRequests } = await import('../../files/request-changes')
  const { shownTurns } = await import('../comments/asks')
  const host = document.createElement('div')
  document.body.append(host)
  const ref64 = 'ab'.repeat(32)
  const picture = { type: 'ref', ref: ref64, mediaType: 'image/png' } as const
  const asked: ShownSideQuestion = {
    ...question, start: 0, end: 1, quote: 'The chart', question: 'Which bar is red?', picture,
    history: heldTranscript(1, [{ start: 0, blocks: [{
      type: 'user', id: 'q1', turnId: 'side', createdAt: '2026-10-18T07:00:01.000Z', model,
      content: [{ type: 'image', source: picture }, { type: 'text', text: 'The user asks this aside…\n\nWhich bar is red?' }],
      preamble: null,
    }] }]),
  }
  const [first] = shownTurns(asked)
  const app = createApp(defineComponent({
    setup: () => {
      provideBlobUrl((source) => `blob:${source}`)
      provideTranscript({ node: 'side', requests: () => storedRequests([]), directory: () => null, pictures: () => [] })
      return () => h(AgentMessageVirtualBlock, { block: first!, isThinkingStreaming: false })
    },
  }))
  app.mount(host)
  unmount = () => app.unmount()
  await nextTick()
  const quoted = host.querySelector<HTMLImageElement>('[data-comment-picture] img')
  expect([quoted?.getAttribute('src'), quoted?.getAttribute('alt')]).toEqual([`blob:${ref64}`, 'The chart'])
  expect(host.textContent).toContain('Which bar is red?')
  expect(host.textContent).not.toContain('aside')
})
