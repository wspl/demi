import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A file tree's rows are objects of their own (`web-application.md`
// § Context menus): a file's menu is the file's, Download where the source
// serves its bytes, and never the tree's Upload Files, which the tree's
// empty space offers. The tree is FileTree over a memory Host on a page of
// happy-dom; a source that takes uploads but serves no bytes is the case.
// Tens of ms, besides the start of the page every happy-dom test pays.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 1280, height: 800 })
await import('../../__tests__/vue-components')
const { createApp, h, nextTick } = await import('vue')
const { createMemoryFileSource, dir, textFile } = await import('../memory-source')
const { default: FileTree } = await import('../FileTree.vue')
const menus = await import('../../ui/context-menu')

let teardown: (() => void)[] = []
afterEach(() => {
  for (const undo of teardown.reverse())
    undo()
  teardown = []
  menus.closeContextMenu()
  document.body.innerHTML = ''
})

test('a file the source serves no bytes of has no menu, never the tree’s Upload Files', async () => {
  const memory = createMemoryFileSource({ platform: 'linux', home: '/w', root: dir({ w: dir({ 'notes.md': textFile('Notes', '2026-09-01T10:00:00Z') }) }) })
  // It takes uploads, and serves no file's bytes.
  const source = { list: memory.list.bind(memory), showListing: memory.showListing.bind(memory), upload: async () => {} }
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp({ render: () => h(FileTree, { source, root: '/w', selected: null }) })
  app.mount(host)
  teardown.push(() => app.unmount())
  const opened: string[][][] = []
  teardown.push(menus.installContextMenus({
    phone: () => false,
    services: { copy: () => {}, copyImage: () => {}, save: () => {}, openLink: () => {} },
    open: (opening) => {
      opened.push(opening.menu.groups().map((group) => group.map((row) => row.label)))
      return false
    },
  }))
  const row = async () => [...host.querySelectorAll<HTMLElement>('[role=tree] *')].find((el) => el.textContent?.trim() === 'notes.md')
  for (let turn = 0; turn < 20 && !(await row()); turn++) {
    await nextTick()
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
  const file = await row()
  expect(file).toBeDefined()
  file!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, button: 2 }))
  host.querySelector('[role=tree]')!.parentElement!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, button: 2 }))
  expect(opened).toEqual([[], [['Upload Files…']]])
})
