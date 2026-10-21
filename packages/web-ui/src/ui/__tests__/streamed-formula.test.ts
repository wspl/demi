import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, jest, test } from 'bun:test'

// A formula that streams into a message is hidden for the 300 ms KaTeX may
// take, alone, and never hides what already shows (`docs/decisions/0033-a-region-draws-once.md`).
// The real component on a happy-dom page, fake timers; a few ms.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, shallowRef } = await import('vue')
const { provideMediaViewer } = await import('../../files/media-viewer')
const { default: StreamedMarkdown } = await import('../StreamedMarkdown.vue')
const { pinRegionForms, resetRegionWaits } = await import('../../infra/region-wait')

let unmount: (() => void) | null = null
beforeEach(() => {
  jest.useFakeTimers()
  resetRegionWaits()
})
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
  pinRegionForms(null)
  jest.useRealTimers()
})

function mountMessage(content: string) {
  const text = shallowRef(content)
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp(defineComponent({
    setup() {
      provideMediaViewer()
      return () => h(StreamedMarkdown, { content: text.value, streaming: false })
    },
  }))
  app.mount(root)
  unmount = () => app.unmount()
  return { root, text }
}

test('a message that has shown keeps showing, and a formula that streams in is hidden alone', async () => {
  // KaTeX is still on its way when the formula arrives.
  pinRegionForms('held')
  const { root, text } = mountMessage('The result is ')
  await nextTick()
  const body = root.querySelector('.markdown-body')!
  expect(body.hasAttribute('data-shown')).toBe(true)
  text.value = 'The result is $x^2$'
  await nextTick()
  expect(body.hasAttribute('data-shown')).toBe(true)
  expect(body.querySelectorAll('.math-waiting')).toHaveLength(1)
})

test('a message whose first render holds a formula has not shown until the formula stops waiting', async () => {
  pinRegionForms('held')
  const { root } = mountMessage('Here it is, $x^2$, as asked.')
  await nextTick()
  const body = root.querySelector('.markdown-body')!
  expect(body.querySelectorAll('.math-waiting')).toHaveLength(1)
  expect(body.hasAttribute('data-shown')).toBe(false)
  // Past 300 ms the formula shows its source, and the message has shown.
  pinRegionForms('plain')
  await nextTick()
  expect(body.querySelectorAll('.math-waiting')).toHaveLength(0)
  expect(body.hasAttribute('data-shown')).toBe(true)
})
