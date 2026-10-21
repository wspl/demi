import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, jest, spyOn, test } from 'bun:test'

// The page's context menus: one handler at the page's root builds each menu from
// what is under the pointer, the content's entries innermost first, then the first
// object's, and opens nothing where nothing declares one.
//
// The handler runs on a page of happy-dom, which lays nothing out: the pointer's
// place is given with each event, and a selection's place by its range's
// rectangles. A long press's half second runs on bun's fake clock, since its timing
// is the behavior under test.
//
// Cost: a few ms a test, besides the start of the page every happy-dom test pays.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 1280, height: 800 })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, withDirectives } = await import('vue')
type App = import('vue').App
const menus = await import('../context-menu')
const { default: ContextMenu } = await import('../ContextMenu.vue')
const { default: SelectionToolbar } = await import('../SelectionToolbar.vue')
const { default: Dialog } = await import('../Dialog.vue')
const { appOverlayStore } = await import('../../overlay/appOverlay')
const { ref } = await import('vue')
type Declaration = import('../context-menu').ContextMenuDeclaration
type Opening = import('../context-menu').ContextMenuOpening

let teardown: (() => void)[] = []
afterEach(() => {
  for (const undo of teardown.reverse())
    undo()
  teardown = []
  menus.closeContextMenu()
  window.getSelection()?.removeAllRanges()
  document.body.innerHTML = ''
  jest.useRealTimers()
})

/** What a row of a test's menu does: it says which row was chosen. */
const chosen: string[] = []
const entry = (label: string) => ({ label, run: () => chosen.push(label) })

/** Mounts `render` on the page, its declarations applied as the product's components apply them. */
function page(render: () => ReturnType<typeof h>, setup?: (app: App) => void): HTMLElement {
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp(defineComponent({ render }))
  setup?.(app)
  app.mount(host)
  teardown.push(() => app.unmount())
  return host
}

const declared = (decl: Declaration | null, ...children: ReturnType<typeof h>[]) =>
  (props: Record<string, unknown>) => withDirectives(h('div', props, children), [[menus.vContextMenu, decl]])

/** A conversation's message: its text with a link, a code block and a field, and its footer's actions. */
function message(extra: Partial<Declaration> = {}) {
  return withDirectives(
    h('div', { id: 'message' }, [
      h('p', { id: 'text' }, ['See ', h('a', { id: 'link', href: 'https://example.com/docs' }, 'the docs'), '.']),
      h('div', { class: 'code-block' }, [h('div', { class: 'code-block-lang', id: 'lang' }, 'sh'), h('pre', [h('code', { id: 'code' }, 'bun run test')])]),
      h('textarea', { id: 'field' }),
      h('button', { id: 'fork-button' }, 'Fork'),
    ]),
    [[menus.vContextMenu, { entries: () => [[entry('Copy Message'), entry('Fork')]], ...extra }]],
  )
}

/** Installs the root handler as the app does; each menu it opens is recorded, and shown as the page shows it. */
function install(phone = false): { opened: Opening[]; services: { copied: unknown[] } } {
  const opened: Opening[] = []
  const services = { copied: [] as unknown[] }
  teardown.push(menus.installContextMenus({
    phone: () => phone,
    services: {
      copy: (source) => services.copied.push(source),
      copyImage: () => {},
      save: () => {},
      openLink: () => {},
    },
    open: (opening) => {
      const shown = menus.showContextMenu(opening)
      if (shown)
        opened.push(opening)
      return shown
    },
  }))
  return { opened, services }
}

const labels = (opening: Opening | undefined) => opening?.menu.groups().map((group) => group.map((row) => row.label))

function rightClick(target: Element, init: MouseEventInit = {}): MouseEvent {
  const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 40, clientY: 50, button: 2, ...init })
  target.dispatchEvent(event)
  return event
}

const byId = (id: string) => document.getElementById(id)!

test('a link in a message has its own entries first, then the message’s, and nothing above the message', () => {
  page(() => declared({ entries: () => [[entry('New Conversation Here')]] }, message())({}))
  const { opened, services } = install()
  const event = rightClick(byId('link'))
  expect(event.defaultPrevented).toBe(true)
  expect(labels(opened[0])).toEqual([['Open Link', 'Copy Link'], ['Copy Message', 'Fork']])
  // A code block's header is the code block's too.
  rightClick(byId('lang'))
  expect(labels(opened[1])).toEqual([['Copy Code'], ['Copy Message', 'Fork']])
  opened[1]!.menu.groups()[0]![0]!.run?.()
  expect(services.copied).toEqual(['bun run test'])
})

test('the selection under the pointer has its own menu: Copy and what its object offers for it, nothing else', () => {
  page(() => message({ selectionEntries: ({ text }) => [entry(`Send “${text}”`)] }))
  const { opened, services } = install()
  const range = document.createRange()
  range.selectNodeContents(byId('link'))
  window.getSelection()!.addRange(range)
  // Where the selection is drawn: the pointer at (40, 50) is on it. Off it, and between its lines, are layout's, which the Chromium test (selection-menu.test.ts) checks.
  // The page reads the selection into a range of its own (`readSelection`), whose boxes are stubbed so.
  const boxes = spyOn(Range.prototype, 'getClientRects').mockImplementation(() => [new DOMRect(30, 40, 60, 20)] as unknown as DOMRectList)
  rightClick(byId('link'))
  boxes.mockRestore()
  expect(labels(opened[0])).toEqual([['Copy', 'Send “the docs”']])
  opened[0]!.menu.groups()[0]![0]!.run?.()
  expect(services.copied).toEqual(['the docs'])
})

test('a picture has Copy Image and Save Image, and an icon, which the page hides as decoration, has none', () => {
  const row = () => withDirectives(
    h('div', { id: 'row' }, [h('img', { id: 'icon', src: '/icons/ts.svg', alt: '', 'aria-hidden': 'true' }), 'cart.ts']),
    [[menus.vContextMenu, () => [[entry('Copy Path')]]]],
  )
  page(() => h('div', [row(), h('img', { id: 'picture', src: '/raw/chart.png', alt: 'chart' })]))
  const { opened } = install()
  rightClick(byId('icon'))
  expect(labels(opened[0])).toEqual([['Copy Path']])
  rightClick(byId('picture'))
  expect(labels(opened[1])).toEqual([['Copy Image', 'Save Image']])
})

test('an object’s menu never holds its container’s', () => {
  page(() => declared({ entries: () => [[entry('Upload Files')]] }, withDirectives(h('div', { id: 'row' }, 'notes.md'), [[menus.vContextMenu, () => [[entry('Download')]]]]))({ id: 'tree' }))
  const { opened } = install()
  rightClick(byId('row'))
  rightClick(byId('tree'))
  expect(opened.map(labels)).toEqual([[['Download']], [['Upload Files']]])
})

test('where nothing declares a menu, none opens, the browser’s included', () => {
  page(() => h('div', [h('h2', { id: 'heading' }, 'Settings'), message()]))
  const { opened } = install()
  expect(rightClick(byId('heading')).defaultPrevented).toBe(true)
  expect(opened.map(labels)).toEqual([])
})

test('a text field, Shift, and a view that takes the event itself keep the browser’s menu', () => {
  page(() => h('div', [message(), declared({ entries: () => [[entry('Reload')]] }, h('canvas', { id: 'remote', onContextmenu: (event: Event) => event.stopPropagation() }))({})]))
  const { opened } = install()
  expect(rightClick(byId('field')).defaultPrevented).toBe(false)
  expect(rightClick(byId('link'), { shiftKey: true }).defaultPrevented).toBe(false)
  rightClick(byId('remote'))
  expect(opened.map(labels)).toEqual([])
})

test('the menu key and Shift-F10 open the focused element’s menu at its corner, once, and Escape gives the focus back', async () => {
  page(() => h('div', [
    h('div', { id: 'list', tabindex: 0, 'aria-activedescendant': 'row-2' }, [
      withDirectives(h('div', { id: 'row-1' }, 'First'), [[menus.vContextMenu, () => [[entry('Rename First')]]]]),
      withDirectives(h('div', { id: 'row-2' }, 'Second'), [[menus.vContextMenu, () => [[entry('Rename Second')]]]]),
    ]),
    h(ContextMenu),
  ]))
  await nextTick()
  byId('row-2').getBoundingClientRect = () => new DOMRect(10, 100, 200, 28)
  const list = byId('list')
  list.focus()
  const key = new KeyboardEvent('keydown', { key: 'F10', shiftKey: true, bubbles: true, cancelable: true })
  list.dispatchEvent(key)
  expect(key.defaultPrevented).toBe(true)
  const shown = menus.shownContextMenu.value
  expect(labels(shown!)).toEqual([['Rename Second']])
  expect([shown!.x, shown!.y]).toEqual([10, 128])
  // The browser's own event for the key, which follows it, opens nothing more.
  rightClick(list)
  expect(menus.shownContextMenu.value).toBe(shown)
  await nextTick()
  await nextTick()
  const menu = document.querySelector<HTMLElement>('[role=menu]')!
  expect(menu.textContent).toContain('Rename Second')
  expect(menu.contains(document.activeElement)).toBe(true)
  menu.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
  await nextTick()
  expect(menus.shownContextMenu.value).toBeNull()
  expect(document.activeElement).toBe(list)

  // A tab declares its own menu: the menu key opens it, and the browser's
  // event for the key that follows on the same tab leaves that menu as it is.
  const tab = byId('row-1')
  tab.tabIndex = 0
  tab.focus()
  tab.dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true, cancelable: true }))
  const opened = menus.shownContextMenu.value
  expect(labels(opened!)).toEqual([['Rename First']])
  rightClick(tab)
  expect(menus.shownContextMenu.value).toBe(opened)
})

/** A touch of one finger at (x, y), as a phone's browser sends it. */
function touch(type: string, target: Element, x: number, y: number): Event {
  const event = new Event(type, { bubbles: true, cancelable: true })
  const fingers = type === 'touchend' ? [] : [{ clientX: x, clientY: y }]
  Object.defineProperty(event, 'touches', { value: fingers })
  target.dispatchEvent(event)
  return event
}

test('a long press opens the menu after half a second still, and its release is no tap', () => {
  jest.useFakeTimers()
  page(() => message())
  const { opened } = install(true)
  const text = byId('text')
  touch('touchstart', text, 100, 200)
  // Half a second, as the design says: a millisecond short of it opens nothing.
  jest.advanceTimersByTime(499)
  expect(opened.map(labels)).toEqual([])
  // A finger that trembles under ten pixels still presses, and another
  // scroller's scroll, as the sidebar revealing its row, leaves the press.
  touch('touchmove', text, 106, 205)
  const elsewhere = document.createElement('div')
  document.body.append(elsewhere)
  elsewhere.dispatchEvent(new Event('scroll'))
  jest.advanceTimersByTime(1)
  expect(labels(opened[0])).toEqual([['Copy Message', 'Fork']])
  expect(touch('touchend', text, 106, 205).defaultPrevented).toBe(true)
})

test('a press that moves or a scroll that starts first is no long press, and a control’s opens nothing', () => {
  jest.useFakeTimers()
  page(() => message())
  const { opened } = install(true)
  const text = byId('text')
  touch('touchstart', text, 100, 200)
  touch('touchmove', text, 100, 211)
  jest.advanceTimersByTime(500)
  touch('touchend', text, 100, 211)
  touch('touchstart', text, 100, 200)
  byId('message').dispatchEvent(new Event('scroll'))
  jest.advanceTimersByTime(500)
  touch('touchend', text, 100, 200)
  touch('touchstart', byId('fork-button'), 100, 300)
  jest.advanceTimersByTime(500)
  expect(touch('touchend', byId('fork-button'), 100, 300).defaultPrevented).toBe(false)
  expect(opened.map(labels)).toEqual([])
})


test('on a phone a message’s text keeps the system’s long press, while a row’s opens its menu', () => {
  jest.useFakeTimers()
  page(() => h('div', [
    message({ text: true }),
    withDirectives(h('div', { id: 'row' }, 'Session cookie rename'), [[menus.vContextMenu, () => [[entry('Rename')]]]]),
  ]))
  const { opened } = install(true)
  touch('touchstart', byId('text'), 100, 200)
  jest.advanceTimersByTime(500)
  expect(touch('touchend', byId('text'), 100, 200).defaultPrevented).toBe(false)
  // Android's own event for the press, which selects the words there, stays the system's.
  expect(rightClick(byId('text')).defaultPrevented).toBe(false)
  touch('touchstart', byId('row'), 100, 300)
  jest.advanceTimersByTime(500)
  touch('touchend', byId('row'), 100, 300)
  expect(opened.map(labels)).toEqual([[['Rename']]])
})

test('on the desktop a right-click on a message’s text still opens the message’s menu', () => {
  page(() => message({ text: true }))
  const { opened } = install(false)
  rightClick(byId('text'))
  expect(opened.map(labels)).toEqual([[['Copy Message', 'Fork']]])
})

/** Selects the text of `node` from `start` to `end`. */
function select(node: Node, start: number, end: number, endNode: Node = node): void {
  const range = document.createRange()
  range.setStart(node, start)
  range.setEnd(endNode, end)
  window.getSelection()!.removeAllRanges()
  window.getSelection()!.addRange(range)
  document.dispatchEvent(new Event('selectionchange'))
}

test('a selection in a message gets a toolbar with what acts on it in place, the right-click menu holding Copy and the rest; any other selection gets none', async () => {
  page(() => h('div', [
    message({ selectionEntries: ({ text }) => [{ ...entry(`Comment on “${text}”`), inPlace: true }, entry(`Send “${text}”`)] }),
    message({ selectionEntries: ({ text }) => [entry(`Send “${text}”`)] }),
    h('p', { id: 'outside' }, 'A heading of the page'),
    h(SelectionToolbar),
  ]))
  await nextTick()
  const [first, second] = document.querySelectorAll('#text')
  select(first!.firstChild!, 0, 3)
  expect(menus.selectionTools(false)?.entries.map((tool) => tool.label)).toEqual(['Comment on “See”'])
  await nextTick()
  const toolbar = () => document.querySelector('[data-selection-toolbar]')
  expect(toolbar()?.textContent?.replace(/\s+/g, ' ').trim()).toBe('Comment on “See”')
  // A phone has no right-click: its toolbar holds every tool the message offers, Copy being the system's.
  expect(menus.selectionTools(true)?.entries.map((tool) => tool.label)).toEqual(['Comment on “See”', 'Send “See”'])
  // Across two messages, in a message that offers nothing in place, or outside any: no toolbar.
  select(first!.firstChild!, 0, 3, second!.firstChild!)
  expect(menus.selectionTools(false)).toBeNull()
  select(second!.firstChild!, 0, 3)
  expect(menus.selectionTools(false)).toBeNull()
  await nextTick()
  expect(toolbar()).toBeNull()
  select(byId('outside').firstChild!, 0, 6)
  expect(menus.selectionTools(true)).toBeNull()
})

test('a context menu opened in a dialog stands on it and leaves it open', async () => {
  const open = ref(true)
  const closes: string[] = []
  page(() => h('div', [
    h(Dialog, { isOpen: open.value, overlayStore: appOverlayStore, label: 'Rename', onClose: () => closes.push('dialog') }, {
      default: () => declared({ entries: () => [[entry('Copy Path')]] }, h('span', 'notes.md'))({ id: 'in-dialog' }),
    }),
    h(ContextMenu),
  ]))
  await nextTick()
  await nextTick()
  const dialogs = appOverlayStore.state.entries.map((each) => each.id)
  expect(dialogs).toHaveLength(1)
  rightClick(byId('in-dialog'))
  await nextTick()
  await nextTick()
  expect(labels(menus.shownContextMenu.value!)).toEqual([['Copy Path']])
  // The menu is a layer on the dialog, not one that replaces it.
  const entries = appOverlayStore.state.entries
  expect(entries.map((each) => [each.layer, each.parent ?? null])).toEqual([[entries[0]!.layer, null], ['nested', dialogs[0]]])
  expect(closes).toEqual([])
  menus.closeContextMenu()
  await nextTick()
  expect(appOverlayStore.state.entries.map((each) => each.id)).toEqual(dialogs)
  open.value = false
  await nextTick()
})

test('an object that goes while its menu shows takes its menu with it, and a long press on one that went opens nothing', async () => {
  jest.useFakeTimers()
  const shown = ref(true)
  const closed: string[] = []
  page(() => h('div', [
    shown.value ? withDirectives(h('div', { id: 'row' }, 'notes.md'), [[menus.vContextMenu, { entries: () => [[entry('Download')]], closed: () => closed.push('row') }]]) : null,
  ]))
  const { opened } = install(true)
  rightClick(byId('row'))
  expect(labels(menus.shownContextMenu.value!)).toEqual([['Download']])
  shown.value = false
  await nextTick()
  expect([menus.shownContextMenu.value, closed]).toEqual([null, ['row']])

  shown.value = true
  await nextTick()
  const row = byId('row')
  touch('touchstart', row, 100, 200)
  shown.value = false
  await nextTick()
  jest.advanceTimersByTime(500)
  expect(opened.map(labels)).toEqual([[['Download']]])
})

test('a row whose action fails tells it as any control’s failure does', async () => {
  const failures: unknown[] = []
  const refused = new Error('The backend refused it.')
  page(() => h('div', [
    withDirectives(h('div', { id: 'row' }, 'notes.md'), [[menus.vContextMenu, () => [[{ label: 'Delete', run: () => Promise.reject(refused) }]]]]),
    h(ContextMenu),
  ]), (app) => {
    app.config.errorHandler = (error) => failures.push(error)
  })
  await nextTick()
  rightClick(byId('row'))
  await nextTick()
  await nextTick()
  document.querySelector<HTMLElement>('[role=menu] [role=menuitem]')!.click()
  await new Promise((resolve) => setTimeout(resolve, 0))
  expect(failures).toEqual([refused])
})
