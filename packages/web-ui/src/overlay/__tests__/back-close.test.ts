import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// Back closes the topmost layer on a phone before it leaves the page: a dialog or
// menu of the overlay store, then the drawer, then the page goes back.
//
// The layers are the frame (SidebarLayout) and components wired as Dialog and
// Popover wire theirs (`useOverlay`), on a happy-dom page 390 px wide. The
// browser's traversal is a task of its own, as a browser queues it.
//
// Cost: a few ms, besides the start of the page every happy-dom test pays.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 390, height: 844 })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, ref } = await import('vue')
const { entriesToTakeOff, historySettled, layersKeptByBack, setHistoryTraversal } = await import('../back-close')
const { createOverlayStore } = await import('../overlayStore')
const { useOverlay } = await import('../../composables/useOverlay')
const { default: SidebarLayout } = await import('../../sidebar/SidebarLayout.vue')

setHistoryTraversal((delta) => setTimeout(() => history.go(delta)))

test('Back closes the layers above the entry it lands on, and every layer when it lands on none of theirs', () => {
  expect(layersKeptByBack(['sheet', 'submenu'], 'sheet')).toBe(1)
  expect(layersKeptByBack(['panel', 'sheet', 'submenu'], 'panel')).toBe(1)
  expect(layersKeptByBack(['sheet'], null)).toBe(0)
  expect(layersKeptByBack(['sheet'], 'gone')).toBe(0)
})

test('layers closed another way take off their entries only from the top, where they are', () => {
  expect(entriesToTakeOff([{ open: true }, { open: false }, { open: false }])).toBe(2)
  // A layer under an open one keeps its entry until the one above closes too.
  expect(entriesToTakeOff([{ open: false }, { open: true }])).toBe(0)
  expect(entriesToTakeOff([])).toBe(0)
})

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

/** A layer of the store, which closes itself when the store or Back asks, as Dialog and Popover do. */
const Layer = defineComponent({
  props: { open: { type: Boolean, required: true }, surface: { type: String as () => 'dialog' | 'popover', required: true } },
  emits: ['close'],
  setup(props, { emit }) {
    const store = createOverlayStore()
    useOverlay(store, () => props.open, () => emit('close'), 'exclusive', props.surface)
    return () => null
  },
})

/** The page at `/conversation`, come from `/previous`, with the drawer, a menu and a dialog, each open or not. */
async function page(open: { drawer: boolean; menu: boolean; dialog: boolean }) {
  history.pushState(null, '', '/previous')
  history.pushState(null, '', '/conversation')
  const drawer = ref(open.drawer)
  const menu = ref(open.menu)
  const dialog = ref(open.dialog)
  const app = createApp(defineComponent(() => () => h(SidebarLayout, {
    open: drawer.value,
    'onUpdate:open': (value: boolean) => {
      drawer.value = value
    },
  }, {
    sidebar: () => h('nav', 'Conversations'),
    default: () => [
      h(Layer, { open: menu.value, surface: 'popover', onClose: () => (menu.value = false) }),
      h(Layer, { open: dialog.value, surface: 'dialog', onClose: () => (dialog.value = false) }),
    ],
  })))
  app.mount(document.body.appendChild(document.createElement('div')))
  unmount = () => app.unmount()
  await nextTick()
  return { drawer, menu, dialog }
}

/** Every traversal under way has landed and the page has drawn what it changed. */
async function settled(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve))
  await historySettled()
  await nextTick()
}

/** The browser's Back, as Android's Back gives it. */
async function back(): Promise<void> {
  const landed = new Promise((resolve) => window.addEventListener('popstate', resolve, { once: true }))
  history.back()
  await landed
  await settled()
}

test('Back closes a dialog over the open drawer first, then the drawer, and only then goes to the previous page', async () => {
  const layers = await page({ drawer: true, menu: false, dialog: false })
  layers.dialog.value = true
  await settled()
  await back()
  expect([layers.dialog.value, layers.drawer.value]).toEqual([false, true])
  await back()
  expect(layers.drawer.value).toBe(false)
  expect(location.pathname).toBe('/conversation')
  await back()
  expect(location.pathname).toBe('/previous')
})

test('a dialog a menu opens as it closes, closed again, leaves no entry that one Back is spent on', async () => {
  // The menu closes first, its entry still being taken off as the dialog opens.
  const closedFirst = await page({ drawer: false, menu: true, dialog: false })
  closedFirst.menu.value = false
  closedFirst.dialog.value = true
  await settled()
  closedFirst.dialog.value = false
  await settled()
  await back()
  expect(location.pathname).toBe('/previous')
  unmount?.()
  unmount = null

  // The dialog opens first, over the menu that closes under it.
  const openedFirst = await page({ drawer: false, menu: true, dialog: false })
  openedFirst.dialog.value = true
  await nextTick()
  openedFirst.menu.value = false
  await settled()
  await back()
  expect(openedFirst.dialog.value).toBe(false)
  await back()
  expect(location.pathname).toBe('/previous')
})
