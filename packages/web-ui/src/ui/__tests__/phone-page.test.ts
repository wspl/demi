import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'
import { installPhonePage } from '../phone-page'

// The phone's page under the on-screen keyboard (`web-application.md` § The
// phone layout): Safari on iOS keeps the page its full height under the
// keyboard and scrolls it to show the focused field, so the app follows the
// visual viewport itself. happy-dom lays nothing out: the page's height, its
// width's switch and the visual viewport are given here as Safari reports
// them on an iPhone 17, 714 px tall, 377 px of it visible with the keyboard
// open. A few ms, besides the start of the page every happy-dom test pays.

GlobalRegistrator.register({ url: 'https://demi.test/' })

/** The phone switch (`phone.ts`) as the turned phone flips it: `turn` changes it and tells the page. */
class Width extends EventTarget {
  matches = true
  turn(phone: boolean): void {
    this.matches = phone
    this.dispatchEvent(new Event('change'))
  }
}

/** The visual viewport as Safari moves it: `move` changes it and tells the page. */
class Viewport extends EventTarget {
  height = 714
  offsetTop = 0
  move(height: number, offsetTop = 0): void {
    this.height = height
    this.offsetTop = offsetTop
    this.dispatchEvent(new Event('resize'))
  }
}

const root = document.documentElement
const viewport = new Viewport()
const width = new Width()
Object.defineProperty(window, 'matchMedia', { value: () => width, configurable: true })
Object.defineProperty(window, 'visualViewport', { value: viewport, configurable: true })
Object.defineProperty(root, 'clientHeight', { value: 714, configurable: true })

let stop: (() => void) | null = null
afterEach(() => {
  stop?.()
  stop = null
  viewport.move(714)
  width.turn(true)
  window.scrollTo(0, 0)
})

/** What the app's frame takes from the page: its height, its top, and whether the keyboard is open. */
function frame() {
  return {
    height: root.style.getPropertyValue('--app-height'),
    top: root.style.getPropertyValue('--app-top'),
    keyboard: 'keyboard' in root.dataset,
  }
}

test('with the keyboard open the app is the visible part of the page, following it as the keyboard changes and closes', () => {
  stop = installPhonePage()
  viewport.move(377)
  expect(frame()).toEqual({ height: '377px', top: '0px', keyboard: true })
  // The emoji keyboard is taller, and Safari pans what is visible a little.
  viewport.move(324, 8)
  expect(frame()).toEqual({ height: '324px', top: '8px', keyboard: true })
  viewport.move(714)
  expect(frame()).toEqual({ height: '', top: '', keyboard: false })
})

test('a phone turned upright with the keyboard open takes the frame, though the visual viewport has not moved', () => {
  // Held sideways the phone is a desktop's width, where the frame is not the page's.
  width.turn(false)
  stop = installPhonePage()
  viewport.move(377)
  expect(frame().keyboard).toBe(false)
  width.turn(true)
  expect(frame()).toEqual({ height: '377px', top: '0px', keyboard: true })
})

test('the page scroll Safari makes to show a focused field goes back to 0, until the page lets go', () => {
  stop = installPhonePage()
  window.scrollTo(0, 337)
  window.dispatchEvent(new Event('scroll'))
  expect(window.scrollY).toBe(0)
  stop()
  stop = null
  window.scrollTo(0, 337)
  window.dispatchEvent(new Event('scroll'))
  expect(window.scrollY).toBe(337)
})
