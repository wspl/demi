import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// Only a press in the sidebar or the conversation's column closes the
// newest of the session's windows; a press in the work panel or on its
// divider closes nothing, so a subagent's changes can be read beside its
// Agents window (`turns.md`, the dock's chips under § Recovering an
// unfinished turn). The frame (SidebarLayout) on a happy-dom page 1280 px
// wide, with a window in the page's stack as the Agents window puts itself
// there: a few ms, besides the start of the page every happy-dom test pays.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 1280, height: 800 })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, ref } = await import('vue')
const { useStackedWindow } = await import('../../ui/window-stack')
const { default: SidebarLayout } = await import('../SidebarLayout.vue')

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

/** A window of the page's stack, as FloatingWindow wires the Agents window. */
const Window = defineComponent({
  props: { open: { type: Boolean, required: true } },
  emits: ['close'],
  setup(props, { emit }) {
    const element = ref<HTMLElement | null>(null)
    useStackedWindow({
      isOpen: () => props.open,
      element: () => element.value,
      otherLayerOpen: () => false,
      close: () => emit('close'),
    })
    return () => props.open ? h('div', { ref: element, id: 'agents' }) : null
  },
})

/** The frame with the sidebar, the transcript, the work panel's Changes tab and the Agents window open over the session. */
async function page() {
  const open = ref(true)
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({
    setup: () => () => h(SidebarLayout, { asideOpen: true }, {
      sidebar: () => h('button', { id: 'sidebar' }),
      default: () => [
        h('div', { id: 'transcript' }),
        h(Window, { open: open.value, onClose: () => { open.value = false } }),
      ],
      aside: () => h('button', { id: 'changes' }),
    }),
  })
  app.mount(root)
  unmount = () => app.unmount()
  await nextTick()
  return {
    open,
    /** Presses `place`, or the work panel's divider, the frame's last. */
    async press(place: string | 'divider') {
      const target = place === 'divider'
        ? [...document.querySelectorAll('[role="separator"]')].at(-1)
        : document.querySelector(place)
      expect(target).toBeDefined()
      expect(target).not.toBeNull()
      target!.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
      await nextTick()
    },
  }
}

test('a press in the work panel or on its divider leaves the window open', async () => {
  const { open, press } = await page()
  await press('#changes')
  await press('divider')
  expect(open.value).toBe(true)
})

test('a press on the transcript or in the sidebar closes the window', async () => {
  const transcript = await page()
  await transcript.press('#transcript')
  expect(transcript.open.value).toBe(false)
  unmount?.()
  document.body.innerHTML = ''
  const sidebar = await page()
  await sidebar.press('#sidebar')
  expect(sidebar.open.value).toBe(false)
})
