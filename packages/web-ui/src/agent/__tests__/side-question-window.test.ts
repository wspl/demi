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
const { EMPTY_TRANSCRIPT } = await import('@demicodes/conversation-client')
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
