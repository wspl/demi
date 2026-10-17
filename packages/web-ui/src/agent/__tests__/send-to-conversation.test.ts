import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, jest, test } from 'bun:test'

// Send to Conversation's submenu (`web-application.md` § Context menus): a
// search field, then New Conversation and the eight conversations the user
// was last active in, leaving out the one shown and the archived ones; what
// the field finds includes the archived ones. The submenu runs on a page of
// happy-dom; the search's pause runs on bun's fake clock. A few ms, besides
// the start of the page every happy-dom test pays.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 1280, height: 800 })
await import('../../__tests__/vue-components')
const { createApp, h, nextTick } = await import('vue')
const { SEARCH_PAUSE_MS } = await import('../../search/search')
const { default: SendToConversationMenu } = await import('../SendToConversationMenu.vue')
type SearchRow = import('../../search/search').SearchRow
type SendTargets = import('../send-to-conversation').SendTargets

afterEach(() => {
  document.body.innerHTML = ''
  jest.useRealTimers()
})

const row = (id: string, minutesAgo: number, archived = false): SearchRow => ({
  conversationId: id,
  title: `Conversation ${id}`,
  titleRanges: [],
  archived,
  lastActiveAt: new Date(Date.UTC(2026, 9, 17, 12, 0) - minutesAgo * 60_000).toISOString(),
  match: null,
})

// Most recent first, as the search window's recent list is: the one shown is the second.
const recent = [row('a', 1), row('shown', 2), row('b', 3), row('old', 4, true), ...['c', 'd', 'e', 'f', 'g', 'h', 'i', 'j'].map((id, index) => row(id, 5 + index))]

function mount() {
  const sent: [string | null, string][] = []
  const searched: string[] = []
  const targets: SendTargets = {
    recent: () => recent,
    search: async (query) => {
      searched.push(query)
      return [row('old', 4, true), row('shown', 2), row('b', 3)]
    },
    // Each as its sidebar row: 'a' runs in the demi project, 'b' failed unread outside any.
    describe: (id) => (id === 'a' ? { status: { status: 'active', unread: false }, project: 'demi' } : id === 'b' ? { status: { status: 'error', unread: true }, project: null } : null),
    shown: () => 'shown',
    send: (to, words) => sent.push([to, words.text]),
  }
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp({ render: () => h(SendToConversationMenu, { targets, words: () => ({ text: 'the cookie rename', files: [] }) }) })
  app.mount(host)
  const rows = () => [...host.querySelectorAll('[data-menu-item]')].map((item) => item.textContent?.trim())
  /** The row of `title`: its project, as its detail, and the tone of its status dot. */
  const look = (title: string) => {
    const item = [...host.querySelectorAll('[data-menu-item]')].find((each) => each.textContent?.includes(title))
    return { project: item?.querySelector('[data-detail]')?.textContent?.trim() ?? null, dot: item?.querySelector('.menu-cell-gutter [data-tone]')?.getAttribute('data-tone') ?? null }
  }
  return { host, rows, look, sent, searched, unmount: () => app.unmount() }
}

test('the submenu lists New Conversation and the eight last active, leaving out the one shown and the archived ones', async () => {
  const menu = mount()
  await nextTick()
  expect(menu.rows()).toEqual(['New Conversation', 'Conversation ademi', ...['b', 'c', 'd', 'e', 'f', 'g', 'h'].map((id) => `Conversation ${id}`)])
  // Drawn as the sidebar draws them: the dot of each one's state, and its project after its title.
  expect([menu.look('Conversation a'), menu.look('Conversation b')]).toEqual([{ project: 'demi', dot: 'accent' }, { project: null, dot: 'danger' }])
  menu.host.querySelector<HTMLElement>('[role=menuitem]')!.click()
  expect(menu.sent).toEqual([[null, 'the cookie rename']])
  menu.unmount()
})

test('what the field finds includes the archived ones, never the one shown', async () => {
  jest.useFakeTimers()
  const menu = mount()
  await nextTick()
  const field = menu.host.querySelector<HTMLInputElement>('input[type=search]')!
  field.value = 'cookie'
  field.dispatchEvent(new Event('input', { bubbles: true }))
  // The field's text reaches the search, which waits for the typing to pause.
  await nextTick()
  await nextTick()
  jest.advanceTimersByTime(SEARCH_PAUSE_MS)
  await Promise.resolve()
  await nextTick()
  expect(menu.searched).toEqual(['cookie'])
  expect(menu.rows()).toEqual(['Conversation oldArchived', 'Conversation b'])
  menu.unmount()
})
