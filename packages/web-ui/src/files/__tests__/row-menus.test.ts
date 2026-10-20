import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A file tree's rows and the commit list's are objects of their own: each row's
// menu is its object's less what the row already is.
//
// - A file's menu is Copy Path, and Download where the source serves its bytes. It
//   is never the tree's Upload Files, which the tree's empty space offers.
// - A commit's menu is Copy Commit Hash.
//
// The tree is FileTree over a memory Host, the list CommitList, on a page of
// happy-dom whose clipboard is stubbed.
//
// Cost: tens of ms, besides the start of the page every happy-dom test pays.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 1280, height: 800 })
await import('../../__tests__/vue-components')
const { createApp, h, nextTick } = await import('vue')
const { createMemoryFileSource, dir, textFile } = await import('../memory-source')
const { default: FileTree } = await import('../FileTree.vue')
const { default: CommitList } = await import('../CommitList.vue')
const menus = await import('../../ui/context-menu')
type Group = readonly import('../../ui/context-menu').ContextMenuEntry[]
type FileBrowserSource = import('../types').FileBrowserSource

const clipboard: string[] = []
Object.defineProperty(navigator, 'clipboard', {
  configurable: true,
  value: { writeText: async (text: string) => void clipboard.push(text) },
})

let teardown: (() => void)[] = []
afterEach(() => {
  for (const undo of teardown.reverse())
    undo()
  teardown = []
  menus.closeContextMenu()
  clipboard.length = 0
  document.body.innerHTML = ''
})

/** The tree of `source` over `/w`, holding `notes.md`; each menu a right-click opens is recorded. */
async function tree(source: Pick<FileBrowserSource, 'list' | 'showListing' | 'contents' | 'upload'>) {
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp({ render: () => h(FileTree, { source, root: '/w', selected: null }) })
  app.mount(host)
  teardown.push(() => app.unmount())
  const opened: Group[][] = []
  teardown.push(menus.installContextMenus({
    phone: () => false,
    services: { copy: () => {}, copyImage: () => {}, save: () => {}, openLink: () => {} },
    open: (opening) => {
      opened.push(opening.menu.groups())
      return false
    },
  }))
  const row = () => [...host.querySelectorAll<HTMLElement>('[role=tree] *')].find((el) => el.textContent?.trim() === 'notes.md')
  for (let turn = 0; turn < 20 && !row(); turn++) {
    await nextTick()
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
  expect(row()).toBeDefined()
  const rightClick = (el: Element) => {
    el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, button: 2 }))
    return opened.at(-1)!
  }
  return { file: () => rightClick(row()!), space: () => rightClick(host.querySelector('[role=tree]')!.parentElement!) }
}

const labels = (groups: Group[]) => groups.map((group) => group.map((row) => row.label))
const memory = () => createMemoryFileSource({ platform: 'linux', home: '/w', root: dir({ w: dir({ 'notes.md': textFile('Notes', '2026-09-01T10:00:00Z') }) }) })

test('a file row has Copy Path and Download, and Copy Path copies the file’s absolute path', async () => {
  const shown = await tree(memory())
  const menu = shown.file()
  expect(labels(menu)).toEqual([['Copy Path', 'Download']])
  await menu.flat().find((row) => row.label === 'Copy Path')!.run!()
  expect(clipboard).toEqual(['/w/notes.md'])
})

test('a commit row has Copy Commit Hash, which copies its full hash, and Uncommitted Changes has no menu', async () => {
  const id = '3f2a9c1e5b7d4a6f8e0c2b4d6a8f0e1c3b5d7a9f'
  const host = document.createElement('div')
  document.body.append(host)
  const commits = [{ id, message: 'Fix the cart total', author: 'Alex', time: '2026-10-18T08:00:00Z' }]
  const app = createApp({ render: () => h(CommitList, { commits, uncommitted: 3, selected: null, more: false, loading: false }) })
  app.mount(host)
  teardown.push(() => app.unmount())
  const opened: Group[][] = []
  teardown.push(menus.installContextMenus({
    phone: () => false,
    services: { copy: () => {}, copyImage: () => {}, save: () => {}, openLink: () => {} },
    open: (opening) => {
      opened.push(opening.menu.groups())
      return false
    },
  }))
  const [uncommitted, commit] = host.querySelectorAll('[role=option]')
  for (const row of [uncommitted!, commit!])
    row.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, button: 2 }))
  expect(opened.map(labels)).toEqual([[['Copy Commit Hash']]])
  await opened[0]!.flat()[0]!.run!()
  expect(clipboard).toEqual([id])
})

test('a file the source serves no bytes of has Copy Path alone, never the tree’s Upload Files', async () => {
  const source = memory()
  // It takes uploads, and serves no file's bytes.
  const shown = await tree({ list: source.list.bind(source), showListing: source.showListing.bind(source), upload: async () => {} })
  expect(labels(shown.file())).toEqual([['Copy Path']])
  expect(labels(shown.space())).toEqual([['Upload Files…']])
})
