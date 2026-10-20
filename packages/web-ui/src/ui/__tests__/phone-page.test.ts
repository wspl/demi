import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'
import { installPhonePage } from '../phone-page'

// The phone's page under the on-screen keyboard: Safari on iOS keeps the page its
// full height under the keyboard and scrolls it to show the focused field, so the app
// follows the visual viewport itself.
//
// happy-dom lays nothing out: the page's height, its width's switch and the visual
// viewport are given here as Safari reports them on an iPhone 17, 714 px tall, 377 px
// of it visible with the keyboard open.
//
// Cost: a few ms, besides the start of the page every happy-dom test pays.

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
  scale = 1
  move(height: number, offsetTop = 0, scale = 1): void {
    this.height = height
    this.offsetTop = offsetTop
    this.scale = scale
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

test('a phone turned sideways, as wide as a desktop, keeps the composer above its keyboard', () => {
  width.turn(false)
  stop = installPhonePage()
  viewport.move(200)
  expect(frame()).toEqual({ height: '200px', top: '0px', keyboard: true })
  width.turn(true)
  viewport.move(377)
  expect(frame()).toEqual({ height: '377px', top: '0px', keyboard: true })
})

test('a page zoomed in by a pinch, as on a desktop, keeps its frame though less of it shows', () => {
  width.turn(false)
  stop = installPhonePage()
  viewport.move(357, 120, 2)
  expect(frame()).toEqual({ height: '', top: '', keyboard: false })
})

test('the page scroll Safari makes to show a focused field goes back to 0, at any width, until the page lets go', () => {
  width.turn(false)
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

// An extension that rewrites the page's viewport tag, as one that drops its
// `maximum-scale=1` does, lets Safari on iOS zoom in on a small field it
// focuses, and the page then stays zoomed with its right edge off the screen:
// the tag comes back as the page shipped it, and last, whatever replaced it.
test('the viewport tag stays as the page shipped it when a script rewrites, removes or outranks it', async () => {
  const SHIPPED = 'width=device-width, initial-scale=1, maximum-scale=1, viewport-fit=cover'
  const tag = document.createElement('meta')
  tag.name = 'viewport'
  tag.content = SHIPPED
  document.head.append(tag)
  stop = installPhonePage()
  // The page's observer answers once the event loop has turned.
  const settled = () => new Promise((resolve) => setImmediate(resolve))
  /** Every viewport tag of the head, in order, by its words. */
  const tags = () => [...document.head.querySelectorAll<HTMLMetaElement>('meta[name="viewport"]')].map((each) => each.content)

  tag.content = 'width=device-width, initial-scale=1'
  await settled()
  expect(tags()).toEqual([SHIPPED])

  tag.remove()
  await settled()
  expect(tags()).toEqual([SHIPPED])

  const other = document.createElement('meta')
  other.name = 'viewport'
  other.content = 'width=device-width'
  document.head.append(other)
  await settled()
  expect(tags()).toEqual(['width=device-width', SHIPPED])

  stop()
  stop = null
  tag.remove()
  other.remove()
})

/** A touch event with one finger at `x`, on `target`, as Safari sends it. */
function finger(target: Element, type: string, x: number, y = 400): TouchEvent {
  const touch = new Touch({ identifier: 0, target, clientX: x, clientY: y })
  const event = new TouchEvent(type, {
    bubbles: true,
    cancelable: true,
    touches: type === 'touchend' ? [] : [touch],
    changedTouches: [touch],
  })
  target.dispatchEvent(event)
  return event
}

// Safari on iOS and the installed app go back or forward when a swipe starts
// at a screen edge: the page prevents such a touch's start, here at 1024 px,
// the width happy-dom gives the page.
test('a touch that starts at either screen edge is kept from the browser, one anywhere else and on a desktop is not', () => {
  stop = installPhonePage()
  const page = document.createElement('div')
  document.body.append(page)
  const prevented = (x: number) => {
    const start = finger(page, 'touchstart', x)
    finger(page, 'touchend', x)
    return start.defaultPrevented
  }
  expect([prevented(4), prevented(16), prevented(17), prevented(512), prevented(1007), prevented(1020)])
    .toEqual([true, true, false, false, false, true])
  width.turn(false)
  expect(prevented(4)).toBe(false)
  page.remove()
})

test('a tap at an edge still clicks what it touched and focuses a field there; a drag from the edge clicks nothing', () => {
  stop = installPhonePage()
  const button = document.createElement('button')
  const field = document.createElement('input')
  document.body.append(button, field)
  let clicks = 0
  button.addEventListener('click', () => {
    clicks += 1
  })

  finger(button, 'touchstart', 6)
  finger(button, 'touchend', 6)
  expect(clicks).toBe(1)

  finger(button, 'touchstart', 6)
  finger(button, 'touchmove', 80)
  finger(button, 'touchend', 80)
  expect(clicks).toBe(1)

  finger(field, 'touchstart', 1018)
  finger(field, 'touchend', 1018)
  expect(document.activeElement).toBe(field)
  button.remove()
  field.remove()
})
