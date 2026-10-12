import { expect, test } from 'bun:test'
import { effectScope, nextTick, ref } from 'vue'
import { useOverlay } from '../../composables/useOverlay'
import { createOverlayStore } from '../overlayStore'
import { usePinnedOpen } from '../pinnedOpen'

// A dialog in a catalog host, as the Dialog wires it: it joins the overlay
// store (and so answers Escape as the top layer) unless it is pinned open.
// Reactive state and an effect scope only; milliseconds.

/** A dialog's layer wiring, the way Dialog.vue does it. */
function dialog(options: { hosted: boolean; open: boolean }) {
  const store = createOverlayStore()
  const isOpen = ref(options.open)
  let id = ''
  const scope = effectScope()
  scope.run(() => {
    const pinned = usePinnedOpen(() => isOpen.value, options.hosted)
    id = useOverlay(store, () => (pinned.value ? false : isOpen.value), () => {})
  })
  return { store, isOpen, isTop: () => store.isTop(id), stop: () => scope.stop() }
}

test('a dialog the user opens in a catalog host is the top layer, so Escape closes it', async () => {
  const live = dialog({ hosted: true, open: false })
  live.isOpen.value = true
  await nextTick()
  expect(live.isTop()).toBe(true)
  live.stop()
})

test('a dialog pinned open in a catalog host owns no layer, until it is closed and opened again', async () => {
  const pinned = dialog({ hosted: true, open: true })
  expect(pinned.store.hasExclusive()).toBe(false)
  pinned.isOpen.value = false
  await nextTick()
  pinned.isOpen.value = true
  await nextTick()
  expect(pinned.isTop()).toBe(true)
  pinned.stop()
})

test('outside a catalog host a dialog open from the start is the top layer', () => {
  const page = dialog({ hosted: false, open: true })
  expect(page.isTop()).toBe(true)
  page.stop()
})
