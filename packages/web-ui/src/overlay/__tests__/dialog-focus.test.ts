import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'
import { effectScope, nextTick, ref } from 'vue'

// A dialog opened on a phone focuses no text field, so the on-screen keyboard does
// not cover what just opened. The panel takes the focus, so the keys and a screen
// reader still start inside it. On the desktop it opens on its first field.
//
// The dialog is wired as Dialog.vue wires it (`useDialogFocus`) on a happy-dom page,
// whose width switch is given here.
//
// Cost: a few milliseconds, besides the start of the page.

GlobalRegistrator.register({ url: 'https://demi.test/' })
const { useDialogFocus } = await import('../dialogFocus')

let phone = false
Object.defineProperty(window, 'matchMedia', {
  value: () => Object.assign(new EventTarget(), { matches: phone }),
  configurable: true,
})

let stop: (() => void) | null = null
afterEach(() => {
  stop?.()
  stop = null
  document.body.replaceChildren()
})

/** Opens a dialog holding one text field, as the page shows it, and answers its panel and field. */
async function openDialogWithField(): Promise<{ panel: HTMLElement, field: HTMLInputElement }> {
  const panel = document.createElement('div')
  panel.setAttribute('role', 'dialog')
  panel.tabIndex = -1
  const field = document.createElement('input')
  panel.append(field)
  const shown = ref<HTMLElement>()
  const scope = effectScope()
  scope.run(() => useDialogFocus({ panel: shown, isOpen: () => true, isTop: () => true }))
  stop = () => scope.stop()
  document.body.append(panel)
  shown.value = panel
  await nextTick()
  return { panel, field }
}

test('a dialog opened on a phone leaves its field for a tap and focuses its panel', async () => {
  phone = true
  const { panel } = await openDialogWithField()
  expect(document.activeElement).toBe(panel)
})

test('a dialog opened on the desktop focuses its first field', async () => {
  phone = false
  const { field } = await openDialogWithField()
  expect(document.activeElement).toBe(field)
})
