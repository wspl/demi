import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A conversation before its first send has the placeholder title, which
// holds no words: the page shows it as New conversation in the language the
// reader has chosen, and changes with it (`product.md` § Conversation
// titles), and a rename from it is an ordinary rename. The row is the
// sidebar's, on a page of happy-dom. A few milliseconds, besides the start
// of the page.

GlobalRegistrator.register({ url: 'http://127.0.0.1:3271/chat' })
await import('../__tests__/vue-components')
const { createApp, h, nextTick, ref } = await import('vue')
const { createPageI18n, setPageLanguage } = await import('@demicodes/web-ui/i18n/text')
const { default: SidebarRow } = await import('@demicodes/web-ui/sidebar/SidebarRow.vue')

const pages: (() => void)[] = []
afterEach(() => {
  pages.splice(0).forEach((unmount) => unmount())
  setPageLanguage('en')
})

test('an untitled conversation reads New conversation in the page\'s language, and a rename from it names it', async () => {
  const i18n = createPageI18n('ja')
  const renaming = ref(false)
  const renamed: string[] = []
  const cancelled: string[] = []
  const element = document.createElement('div')
  document.body.append(element)
  const app = createApp({
    render: () => h(SidebarRow, {
      conversation: { id: 'c1', title: null, updatedAt: '2026-10-15T00:00:00.000Z', status: 'idle', projectId: null, pinned: false, unread: false },
      open: true,
      selected: true,
      focused: false,
      menuOpen: false,
      renaming: renaming.value,
      onRenameSubmit: (title: string) => renamed.push(title),
      onRenameCancel: () => cancelled.push('cancel'),
    }),
  })
  app.use(i18n)
  app.mount(element)
  pages.push(() => app.unmount())

  expect(element.textContent).toContain('新しいチャット')
  setPageLanguage('en')
  await nextTick()
  expect(element.textContent).toContain('New conversation')
  expect(element.textContent).not.toContain('新しいチャット')

  // The field starts with the words shown; leaving them as they are is no
  // rename, and other words are one.
  renaming.value = true
  await nextTick()
  const field = element.querySelector('input')!
  expect(field.value).toBe('New conversation')
  field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
  expect(renamed).toEqual([])
  expect(cancelled).toEqual(['cancel'])
  renaming.value = false
  await nextTick()
  renaming.value = true
  await nextTick()
  const again = element.querySelector('input')!
  again.value = 'Plans'
  again.dispatchEvent(new Event('input', { bubbles: true }))
  again.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
  expect(renamed).toEqual(['Plans'])
})
