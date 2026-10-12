import { expect, test } from 'bun:test'
import { effectScope, nextTick, ref } from 'vue'
import { useOverlay } from '../../composables/useOverlay'
import { createOverlayStore } from '../overlayStore'
import { usePinnedOpen } from '../pinnedOpen'

// A dialog or popover in a catalog host joins the overlay store (and so answers
// Escape as the top layer) unless it is pinned open as the host renders.
// Reactive state and an effect scope only; milliseconds.

/** A dialog's or popover's layer wiring, the way Dialog.vue and Popover.vue do it. */
function surface(options: { host: 'none' | 'mounting' | 'mounted'; open: boolean }) {
  const store = createOverlayStore()
  const isOpen = ref(options.open)
  // A host's element exists only once the host has mounted, after its content is set up.
  const host = ref<object | undefined>(options.host === 'mounted' ? {} : undefined)
  let id = ''
  const scope = effectScope()
  scope.run(() => {
    const pinned = usePinnedOpen(() => isOpen.value, options.host === 'none' ? null : host)
    id = useOverlay(store, () => (pinned.value ? false : isOpen.value), () => {})
  })
  return {
    store,
    isOpen,
    mountHost: () => (host.value = {}),
    isTop: () => store.isTop(id),
    stop: () => scope.stop(),
  }
}

test('a surface the user opens in a catalog host is the top layer, so Escape closes it', async () => {
  const live = surface({ host: 'mounting', open: false })
  live.mountHost()
  live.isOpen.value = true
  await nextTick()
  expect(live.isTop()).toBe(true)
  live.stop()
})

test('a surface mounted open after the host stands, as a context menu keyed to each opening is, is the top layer', () => {
  const menu = surface({ host: 'mounted', open: true })
  expect(menu.isTop()).toBe(true)
  menu.stop()
})

test('a surface pinned open as the host first renders owns no layer, until it is closed and opened again', async () => {
  const pinned = surface({ host: 'mounting', open: true })
  pinned.mountHost()
  expect(pinned.store.hasExclusive()).toBe(false)
  pinned.isOpen.value = false
  await nextTick()
  pinned.isOpen.value = true
  await nextTick()
  expect(pinned.isTop()).toBe(true)
  pinned.stop()
})

test('outside a catalog host a surface open from the start is the top layer', () => {
  const page = surface({ host: 'none', open: true })
  expect(page.isTop()).toBe(true)
  page.stop()
})
