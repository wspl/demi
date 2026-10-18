import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// The browser tab carries the sidebar's news (`conversations.md` §
// Conversations and projects): the title counts the conversations whose dot
// is red, yellow or green, one count of one kind or of all as to do; the
// icon wears the most urgent dot; an installed app's badge carries the
// number; the conversation open in a page in front counts nothing; coming
// forward clears it all, and the badge follows the page that came forward
// last. The page is happy-dom's, whose focus and visibility the test turns.
// A few ms each, besides the start of the page.

GlobalRegistrator.register({ url: 'https://demi.test/chat' })
const { nextTick, ref } = await import('vue')
const { createPageI18n, setPageLanguage } = await import('../../i18n/text')
const { followTabAttention } = await import('../tab-attention')
createPageI18n('en')

type Row = { id: string; status: 'idle' | 'active' | 'done' | 'error' | 'aborted'; unread: boolean; needsYou?: boolean }

const tokens = document.createElement('style')
tokens.textContent = `
  html[data-theme="light"] { --on-danger: #dc2626; --on-attention: #ca8a04; --on-success: #16a34a; --surface-base: #f4f4f4; }
  html[data-theme="dark"] { --on-danger: #f87171; --on-attention: #fde047; --on-success: #34d399; --surface-base: #1a1a1a; }
`
document.head.append(tokens)
document.documentElement.dataset.theme = 'light'
Object.defineProperty(window, 'matchMedia', {
  value: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }),
  configurable: true,
})
const icon = document.createElement('link')
icon.rel = 'icon'
const plainIcon = `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="100 170 1000 1000"><rect x="100" y="170" width="1000" height="1000"/></svg>')}`
icon.setAttribute('href', plainIcon)
document.head.append(icon)

/** The page's focus and visibility, which the test turns as a user would. */
let focused = false
let visible = false
Object.defineProperty(document, 'hasFocus', { value: () => focused, configurable: true })
Object.defineProperty(document, 'visibilityState', { get: () => (visible ? 'visible' : 'hidden'), configurable: true })
function bringForward(): void {
  visible = true
  document.dispatchEvent(new Event('visibilitychange'))
  focused = true
  window.dispatchEvent(new Event('focus'))
}
function sendBack(): void {
  focused = false
  window.dispatchEvent(new Event('blur'))
  visible = false
  document.dispatchEvent(new Event('visibilitychange'))
}

/** What the installed app's badge was last set to; 0 once cleared. */
const badges: number[] = []
Object.defineProperty(navigator, 'setAppBadge', { value: (count: number) => (badges.push(count), Promise.resolve()), configurable: true })
Object.defineProperty(navigator, 'clearAppBadge', { value: () => (badges.push(0), Promise.resolve()), configurable: true })

/** The color of the dot the tab's icon wears, or null for the plain icon. */
function iconDot(): { fill: string; ring: string } | null {
  const href = document.head.querySelector('link[rel="icon"]')?.getAttribute('href') ?? ''
  if (href === plainIcon)
    return null
  const svg = new DOMParser().parseFromString(decodeURIComponent(href.replace('data:image/svg+xml,', '')), 'image/svg+xml')
  const [ring, fill] = [...svg.querySelectorAll('circle')].map((circle) => circle.getAttribute('fill') ?? '')
  return { fill: fill ?? '', ring: ring ?? '' }
}

/** Waits until the icon wears `expected`; the deadline only guards against a hang. */
async function iconShows(expected: { fill: string; ring: string } | null): Promise<void> {
  const deadline = Date.now() + 2000
  while (JSON.stringify(iconDot()) !== JSON.stringify(expected)) {
    if (Date.now() > deadline)
      throw new Error(`the icon wears ${JSON.stringify(iconDot())}, not ${JSON.stringify(expected)}`)
    await new Promise((resolve) => setImmediate(resolve))
  }
}

const releases: (() => void)[] = []
afterEach(() => {
  releases.splice(0).forEach((release) => release())
  setPageLanguage('en')
  badges.splice(0)
  focused = false
  visible = false
})

function page(rows: Row[], openId: string | null = null, openTitle: string | null = null) {
  const conversations = ref(rows)
  const open = ref(openId)
  const title = ref(openTitle)
  releases.push(followTabAttention({ conversations: () => conversations.value, openId: () => open.value, openTitle: () => title.value }))
  return { conversations, open, title }
}

test('the title counts one kind by its name and several kinds as to do, in the page\'s language', async () => {
  const { conversations, title } = page([])
  expect(document.title).toBe('demi')
  title.value = '整理季度报告'
  await nextTick()
  expect(document.title).toBe('整理季度报告 · demi')

  conversations.value = [
    { id: 'a', status: 'done', unread: true },
    { id: 'b', status: 'done', unread: true },
    { id: 'c', status: 'done', unread: false },
    { id: 'd', status: 'active', unread: false },
  ]
  await nextTick()
  expect(document.title).toBe('(2 done) 整理季度报告 · demi')
  conversations.value = [{ id: 'e', status: 'error', unread: true }]
  await nextTick()
  expect(document.title).toBe('(1 failed) 整理季度报告 · demi')
  conversations.value = [{ id: 'f', status: 'idle', unread: false, needsYou: true }]
  await nextTick()
  expect(document.title).toBe('(1 to do) 整理季度报告 · demi')
  conversations.value = [
    { id: 'a', status: 'done', unread: true },
    { id: 'e', status: 'error', unread: true },
    { id: 'f', status: 'done', unread: true, needsYou: true },
    { id: 'g', status: 'done', unread: true },
  ]
  await nextTick()
  expect(document.title).toBe('(4 to do) 整理季度报告 · demi')

  setPageLanguage('zh-Hans')
  await nextTick()
  expect(document.title).toBe('(4 个待办) 整理季度报告 · demi')
  conversations.value = [{ id: 'e', status: 'error', unread: true }]
  await nextTick()
  expect(document.title).toBe('(1 个出错) 整理季度报告 · demi')
  setPageLanguage('ja')
  await nextTick()
  expect(document.title).toBe('(失敗 1 件) 整理季度报告 · demi')
})

test('the icon wears the most urgent dot, red over yellow over green, ringed in the theme\'s background', async () => {
  const { conversations } = page([{ id: 'a', status: 'done', unread: true }])
  await iconShows({ fill: '#16a34a', ring: '#f4f4f4' })
  conversations.value = [...conversations.value, { id: 'b', status: 'idle', unread: false, needsYou: true }]
  await iconShows({ fill: '#ca8a04', ring: '#f4f4f4' })
  conversations.value = [...conversations.value, { id: 'c', status: 'error', unread: true }]
  await iconShows({ fill: '#dc2626', ring: '#f4f4f4' })
  document.documentElement.dataset.theme = 'dark'
  await iconShows({ fill: '#f87171', ring: '#1a1a1a' })
  document.documentElement.dataset.theme = 'light'
  conversations.value = [{ id: 'a', status: 'done', unread: false }]
  await iconShows(null)
})

test('the conversation open in a page in front counts nothing, and counts once the page is behind', async () => {
  const { conversations } = page([{ id: 'open', status: 'active', unread: false }, { id: 'other', status: 'active', unread: false }], 'open', 'Open one')
  bringForward()
  conversations.value = [{ id: 'open', status: 'error', unread: true }, { id: 'other', status: 'done', unread: true }]
  await nextTick()
  expect(document.title).toBe('(1 done) Open one · demi')
  sendBack()
  await nextTick()
  expect(document.title).toBe('(2 to do) Open one · demi')
})

test('coming forward clears the count, the icon\'s dot and the badge, and only a dot lit after it counts again', async () => {
  const { conversations } = page([{ id: 'a', status: 'done', unread: true }, { id: 'b', status: 'active', unread: false }])
  expect(document.title).toBe('(1 done) demi')
  await iconShows({ fill: '#16a34a', ring: '#f4f4f4' })

  bringForward()
  await nextTick()
  expect(document.title).toBe('demi')
  await iconShows(null)
  expect(badges.at(-1)).toBe(0)
  sendBack()
  // b fails after the user looked; a stays green, as it was then.
  conversations.value = [{ id: 'a', status: 'done', unread: true }, { id: 'b', status: 'error', unread: true }]
  await nextTick()
  expect(document.title).toBe('(1 failed) demi')
  expect(badges.at(-1)).toBe(1)
  // a runs again and finishes: its dot lights anew.
  conversations.value = [{ id: 'a', status: 'active', unread: false }, { id: 'b', status: 'error', unread: true }]
  await nextTick()
  conversations.value = [{ id: 'a', status: 'done', unread: true }, { id: 'b', status: 'error', unread: true }]
  await nextTick()
  expect(document.title).toBe('(2 to do) demi')
  expect(badges.at(-1)).toBe(2)
})

test('the badge follows the page that came forward last', async () => {
  const { conversations } = page([{ id: 'a', status: 'active', unread: false }, { id: 'b', status: 'active', unread: false }])
  bringForward()
  sendBack()
  conversations.value = [{ id: 'a', status: 'done', unread: true }, { id: 'b', status: 'active', unread: false }]
  await nextTick()
  expect(badges.at(-1)).toBe(1)
  // Another page of the user's comes forward and says so. A channel delivers to
  // its listeners in the order they were made, so once the witness, made after
  // this page's, has the message, this page has it too.
  const other = new BroadcastChannel('demi-tab-forward')
  const witness = new BroadcastChannel('demi-tab-forward')
  const delivered = new Promise((resolve) => witness.addEventListener('message', resolve, { once: true }))
  other.postMessage(null)
  await delivered
  other.close()
  witness.close()
  const written = badges.length
  conversations.value = [{ id: 'a', status: 'done', unread: true }, { id: 'b', status: 'error', unread: true }]
  await nextTick()
  expect(document.title).toBe('(2 to do) demi')
  expect(badges.length).toBe(written)
})
