import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// The agent's activity over a live view (`mobile.md` § The live view): while
// its input is delivered, a note says so and a ring marks where it acted;
// once it is done the note leaves and the ring fades where it was. One Vue
// app on a happy-dom page; a few ms.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, h, nextTick, reactive } = await import('vue')
const { default: LiveActivityOverlay } = await import('../LiveActivityOverlay.vue')

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

test('the note shows while the agent acts, and the ring stays where it acted', async () => {
  const props = reactive({ active: false, label: 'The agent is using this device', mark: null as { x: number; y: number } | null })
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({ render: () => h(LiveActivityOverlay, props) })
  app.mount(root)
  unmount = () => app.unmount()
  const note = () => root.querySelector('[role="status"]')
  const ring = () => root.querySelector<HTMLElement>('.live-activity-mark')

  expect(note()).toBeNull()
  expect(ring()).toBeNull()

  props.active = true
  props.mark = { x: 120, y: 340 }
  await nextTick()
  expect(note()?.textContent?.trim()).toBe('The agent is using this device')
  expect([ring()?.style.left, ring()?.style.top]).toEqual(['120px', '340px'])
  expect(ring()?.classList.contains('live-activity-mark--idle')).toBe(false)

  props.active = false
  await nextTick()
  // The leaving note goes once its transition ends, a frame or two later.
  await new Promise<void>((resolve) => {
    const gone = new MutationObserver(() => {
      if (!note()) {
        gone.disconnect()
        resolve()
      }
    })
    gone.observe(root, { childList: true, subtree: true })
  })
  expect(note()).toBeNull()
  expect(ring()?.classList.contains('live-activity-mark--idle')).toBe(true)
})
