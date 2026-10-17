import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { expect, test } from 'bun:test'
import { followThemeColor } from '../themeColor'

// The status bar's band on an iPhone takes the page's `theme-color`, in Safari
// and in the installed app: it was the base surface's gray over the white bar
// of a conversation, two tones at the top of the screen. The page's tag follows
// the surface at its top as the theme, the tone and the width change. The
// tokens are given here as product-appearance.css sets them; happy-dom lays
// nothing out. A few ms, besides the start of the page every happy-dom test pays.

GlobalRegistrator.register({ url: 'https://demi.test/' })

/** The phone switch (`phone.ts`): `turn` changes it and tells the page. */
class Width extends EventTarget {
  matches = true
  turn(phone: boolean): void {
    this.matches = phone
    this.dispatchEvent(new Event('change'))
  }
}

test('the page\'s theme color is the surface at its top, as the theme, the tone and the width change', async () => {
  const width = new Width()
  Object.defineProperty(window, 'matchMedia', { value: () => width, configurable: true })
  const tokens = document.createElement('style')
  tokens.textContent = `
    html[data-theme="light"] { --surface-base: #f4f4f4; --surface: #ffffff; }
    html[data-theme="dark"] { --surface-base: #1a1a1a; --surface: #242424; }
    html[data-theme="light"][data-tone="warm"] { --surface-base: #f8f5ef; --surface: #fffdf9; }
  `
  document.head.append(tokens)
  // As the page's HTML names them, by width and scheme, for the moment before the app runs.
  for (const [content, media] of [['#ffffff', '(prefers-color-scheme: light) and (max-width: 767px)'], ['#f4f4f4', '(prefers-color-scheme: light)']]) {
    const tag = document.createElement('meta')
    tag.name = 'theme-color'
    tag.content = content
    tag.media = media
    document.head.append(tag)
  }
  const root = document.documentElement
  root.dataset.theme = 'light'
  root.dataset.tone = 'ink'
  // The page's observer answers once the event loop has turned.
  const settled = () => new Promise((resolve) => setImmediate(resolve))
  /** Every theme-color tag of the head: its color and its query. */
  const tags = () => [...document.head.querySelectorAll<HTMLMetaElement>('meta[name="theme-color"]')]
    .map((tag) => ({ color: tag.content, media: tag.getAttribute('media') }))

  const stop = followThemeColor()
  expect(tags()).toEqual([{ color: '#ffffff', media: null }])

  // The user picks Dark while the system is light: the tag follows the page, not the system.
  root.dataset.theme = 'dark'
  await settled()
  expect(tags()).toEqual([{ color: '#242424', media: null }])

  root.dataset.theme = 'light'
  root.dataset.tone = 'warm'
  await settled()
  expect(tags()).toEqual([{ color: '#fffdf9', media: null }])

  // A window wider than a phone's has the sidebar at its top.
  width.turn(false)
  expect(tags()).toEqual([{ color: '#f8f5ef', media: null }])

  stop()
  root.dataset.theme = 'dark'
  await settled()
  expect(tags()).toEqual([{ color: '#f8f5ef', media: null }])
  tokens.remove()
})
