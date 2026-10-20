import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { beforeEach, expect, test } from 'bun:test'

// A button-like element that is not a native button presses as one does: Return
// as it goes down, Space as it comes up, never while it is aria-disabled. A
// page of happy-dom; a few milliseconds.

GlobalRegistrator.register({ url: 'https://demi.test/' })
// After the page, since the handler asks for HTMLElement.
const { pressOnKey } = await import('../button-events')

let element: HTMLElement
let clicks: number

beforeEach(() => {
  document.body.replaceChildren()
  element = document.createElement('span')
  element.setAttribute('role', 'button')
  element.tabIndex = 0
  element.addEventListener('keydown', pressOnKey)
  element.addEventListener('keyup', pressOnKey)
  clicks = 0
  element.addEventListener('click', () => clicks++)
  document.body.append(element)
})

/** Sends a key to the element and says whether the page's default (scrolling, for Space) was left to happen. */
function press(type: 'keydown' | 'keyup', key: string): boolean {
  const event = new KeyboardEvent(type, { key, bubbles: true, cancelable: true })
  element.dispatchEvent(event)
  return !event.defaultPrevented
}

test('Return clicks as it goes down', () => {
  press('keydown', 'Enter')
  expect(clicks).toBe(1)
  press('keyup', 'Enter')
  expect(clicks).toBe(1)
})

test('Space clicks as it comes up, and the page does not scroll as it goes down', () => {
  expect(press('keydown', ' ')).toBe(false)
  expect(clicks).toBe(0)
  press('keyup', ' ')
  expect(clicks).toBe(1)
})

test('a Space that went down on another element does not click this one as it comes up', () => {
  press('keyup', ' ')
  expect(clicks).toBe(0)
})

test('an aria-disabled element is not pressed', () => {
  element.setAttribute('aria-disabled', 'true')
  press('keydown', 'Enter')
  press('keydown', ' ')
  press('keyup', ' ')
  expect(clicks).toBe(0)
})

test('keys a button does not take pass through, Tab among them', () => {
  expect(press('keydown', 'Tab')).toBe(true)
  expect(press('keydown', 'a')).toBe(true)
  expect(clicks).toBe(0)
})
