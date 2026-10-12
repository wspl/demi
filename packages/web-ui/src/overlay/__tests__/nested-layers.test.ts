import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { expect, test } from 'bun:test'
import { effectScope, nextTick, ref } from 'vue'

// Escape closes the innermost open layer first, as macOS does: a dropdown
// opened in a dialog closes and the dialog stays, and the next Escape closes
// the dialog. The layers are wired as Dialog and Popover wire them (the
// overlay store, `useOverlay`, `useEscapeClose`), with a real Escape key on a
// happy-dom page. A few milliseconds, besides the start of the page.

GlobalRegistrator.register({ url: 'https://demi.test/' })
const { createOverlayStore } = await import('../overlayStore')
const { useOverlay } = await import('../../composables/useOverlay')
const { useEscapeClose } = await import('../../composables/useEscapeClose')

/** Wires a layer the way its component does, closing itself through `isOpen` when the store or Escape asks. */
function layer(store: ReturnType<typeof createOverlayStore>, kind: 'exclusive' | 'nested', surface: 'dialog' | 'popover', parent?: string) {
  const isOpen = ref(true)
  const close = () => (isOpen.value = false)
  const id = useOverlay(store, () => isOpen.value, close, kind, surface, parent)
  useEscapeClose(() => isOpen.value && store.isTop(id), close)
  return { id, isOpen }
}

function pressEscape(): void {
  window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
}

/** A dialog with a dropdown open in it, and what the page shows. */
function dialogWithDropdown() {
  const store = createOverlayStore()
  const scope = effectScope()
  const layers = scope.run(() => {
    const dialog = layer(store, 'exclusive', 'dialog')
    const dropdown = layer(store, 'nested', 'popover', dialog.id)
    return { dialog, dropdown }
  })!
  return { store, ...layers, stop: () => scope.stop() }
}

test('Escape closes a dropdown open in a dialog and leaves the dialog; the next one closes the dialog', async () => {
  const page = dialogWithDropdown()
  pressEscape()
  await nextTick()
  expect([page.dropdown.isOpen.value, page.dialog.isOpen.value]).toEqual([false, true])
  pressEscape()
  await nextTick()
  expect(page.dialog.isOpen.value).toBe(false)
  page.stop()
})

test('closing the dialog takes the dropdown in it along', () => {
  const page = dialogWithDropdown()
  page.store.remove(page.dialog.id)
  expect(page.dropdown.isOpen.value).toBe(false)
  expect(page.store.hasExclusive()).toBe(false)
  page.stop()
})

test('a menu opened elsewhere replaces the dropdown, as it replaces any menu', () => {
  const page = dialogWithDropdown()
  page.store.push('elsewhere', () => {})
  expect(page.dropdown.isOpen.value).toBe(false)
  page.stop()
})
