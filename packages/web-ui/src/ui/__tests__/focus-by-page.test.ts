import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { beforeEach, expect, test } from 'bun:test'

// The focus ring shows only where the keyboard moved the focus
// (`web-application.md` § Keyboard): a focus the page moves, giving it back to
// what opened a layer or taking it on opening, says so to the browser
// (`focusVisible: false`), whatever key closed the layer. A page of happy-dom;
// a few milliseconds.

GlobalRegistrator.register({ url: 'https://demi.test/' })
// After the page, since the modules ask for HTMLElement.
const { useFocusReturn } = await import('../../overlay/focusReturn')
const { useAutofocus } = await import('../autofocus')

let opener: HTMLElement
let panel: HTMLElement
/** The options of every `focus()` call, by the element it was called on. */
let focused: { id: string, options: FocusOptions | undefined }[]

function button(id: string): HTMLElement {
  const element = document.createElement('button')
  element.id = id
  const focus = element.focus.bind(element)
  element.focus = (options?: FocusOptions) => {
    focused.push({ id, options })
    focus(options)
  }
  document.body.append(element)
  return element
}

beforeEach(() => {
  document.body.replaceChildren()
  focused = []
  opener = button('opener')
  panel = button('panel')
})

test('a closed layer gives the focus back without a ring', () => {
  const returning = useFocusReturn((active) => active === panel)
  opener.focus()
  returning.open()
  panel.focus()
  focused = []
  returning.close()
  expect(focused).toEqual([{ id: 'opener', options: { preventScroll: true, focusVisible: false } }])
})

test('a field the page focuses on opening takes the focus without a ring', () => {
  const autofocus = useAutofocus()
  autofocus(panel, { preventScroll: true })
  expect(focused).toEqual([{ id: 'panel', options: { preventScroll: true, focusVisible: false } }])
})

test('a surface behind a dialog never takes the focus from it, one in the dialog does', () => {
  const dialog = document.createElement('div')
  dialog.setAttribute('role', 'dialog')
  document.body.append(dialog)
  const inside = document.createElement('button')
  dialog.append(inside)
  const autofocus = useAutofocus()
  inside.focus()
  // The composer's late focus, behind the dialog that opened first.
  autofocus(opener)
  expect(document.activeElement).toBe(inside)
  // A field of the dialog itself, or of one stacked on it, is not behind it.
  const field = document.createElement('input')
  dialog.append(field)
  autofocus(field)
  expect(document.activeElement).toBe(field)
})
