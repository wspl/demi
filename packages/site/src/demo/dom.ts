// How a scene waits for the page: until an element the product renders
// shows, or until its own state holds.
import type { Conversation } from './conversation'

/** The element `selector` finds once the page shows it, polling each frame for a few seconds. */
export async function shown<T extends Element = HTMLElement>(c: Conversation, selector: string, tries = 180): Promise<T> {
  for (let attempt = 0; attempt < tries; attempt++) {
    const element = document.querySelector<T>(selector)
    if (element)
      return element
    await c.wait(16)
  }
  throw new Error(`The demo never showed ${selector}`)
}

/** The first element `selector` matches whose text holds `text`, once one shows. */
export async function shownText(c: Conversation, selector: string, text: string): Promise<HTMLElement> {
  for (let attempt = 0; attempt < 180; attempt++) {
    const element = [...document.querySelectorAll<HTMLElement>(selector)].find((each) => each.textContent?.includes(text))
    if (element)
      return element
    await c.wait(16)
  }
  throw new Error(`The demo never showed ${text} in ${selector}`)
}

/** Waits until `holds` does. */
export async function until(c: Conversation, holds: () => boolean): Promise<void> {
  while (!holds())
    await c.wait(16)
}

/** Presses `element` as a mouse does, down then up then click, for a control that opens on its press. */
export function press(element: HTMLElement): void {
  const { x, y, width, height } = element.getBoundingClientRect()
  const at = { bubbles: true, cancelable: true, clientX: x + width / 2, clientY: y + height / 2, button: 0 }
  element.dispatchEvent(new PointerEvent('pointerdown', { ...at, pointerType: 'mouse', isPrimary: true }))
  element.dispatchEvent(new MouseEvent('mousedown', at))
  element.dispatchEvent(new PointerEvent('pointerup', { ...at, pointerType: 'mouse', isPrimary: true }))
  element.dispatchEvent(new MouseEvent('mouseup', at))
  element.dispatchEvent(new MouseEvent('click', at))
}
