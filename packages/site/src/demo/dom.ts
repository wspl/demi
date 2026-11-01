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

/**
 * Opens the newest row of the transcript whose text holds `text`, a call's
 * or a group of calls', unless it shows open already, as a click on its face does.
 */
export async function openRow(c: Conversation, text: string): Promise<void> {
  const rows = () => [...document.querySelectorAll<HTMLElement>('[data-block-id] button[aria-expanded]')]
    .filter((button) => button.parentElement?.textContent?.includes(text))
  await until(c, () => rows().length > 0)
  const button = rows().at(-1)!
  if (button.getAttribute('aria-expanded') !== 'true')
    press(button.parentElement!)
}

/**
 * Scrolls the transcript to its end, smoothly, as a reader does after opening
 * a row: a row the reader opens holds the view where it stands, and the list
 * follows its end again once the reader is back there.
 */
export function scrollToEnd(): void {
  const list = document.querySelector('[data-block-id]')?.closest<HTMLElement>('.scroll-area-viewport')
  list?.scrollTo({ top: list.scrollHeight, behavior: 'smooth' })
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
