import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterAll, afterEach, expect, test } from 'bun:test'

// The File view's tree follows the file it shows (`file-previews.md` § The
// File view's bars): whenever the view shows a file, however it came, the
// folders above it unfold, its row is selected and brought into view; a
// file opened while another's folders still list wins; a file outside the
// tree's root leaves no row selected. FileTree over a memory Host whose
// listings can be held, on a page of happy-dom, which lays nothing out: each
// row stands one pitch under the last and the tree's view is 200 px tall.
// Tens of ms, besides the start of the page every happy-dom test pays.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 1280, height: 800 })
await import('../../__tests__/vue-components')
const { createApp, h, nextTick, ref } = await import('vue')
const { deferred } = await import('@demicodes/utils')
const { dir, memoryFileReads, textFile } = await import('../memory-source')
const { keptSource } = await import('../kept-source')
const { HostFiles } = await import('../file-cache')
const { TREE_ROW_PITCH_PX } = await import('../tree')
const { default: FileTree } = await import('../FileTree.vue')

const VIEW_PX = 200
// Under the caption's room and the viewport's padding, as Tree lays them out.
const FIRST_ROW_PX = 4 + TREE_ROW_PITCH_PX

/** Gives `name` on every HTML element the getter `get`; the returned function puts back what was there. */
function stub(name: 'offsetTop' | 'clientHeight', get: (this: HTMLElement) => number): () => void {
  const own = Object.getOwnPropertyDescriptor(HTMLElement.prototype, name)
  Object.defineProperty(HTMLElement.prototype, name, { configurable: true, get })
  return () => {
    if (own)
      Object.defineProperty(HTMLElement.prototype, name, own)
    else
      Reflect.deleteProperty(HTMLElement.prototype, name)
  }
}

const unstub = [
  stub('offsetTop', function () {
    if (this.getAttribute('role') !== 'treeitem' || this.parentElement?.getAttribute('role') !== 'tree')
      return 0
    const rows = [...this.parentElement.children].filter((el) => el.getAttribute('role') === 'treeitem')
    return FIRST_ROW_PX + rows.indexOf(this) * TREE_ROW_PITCH_PX
  }),
  stub('clientHeight', function () {
    return this.classList.contains('scroll-area-viewport') ? VIEW_PX : 0
  }),
]
afterAll(() => {
  for (const undo of unstub)
    undo()
})

let teardown: (() => void)[] = []
afterEach(() => {
  for (const undo of teardown.reverse())
    undo()
  teardown = []
  document.body.innerHTML = ''
})

const at = '2026-09-01T10:00:00Z'
// Thirty folders at the top, which list first, so a folder named after them lies far below the view.
const many = Object.fromEntries(Array.from({ length: 30 }, (_, index) => [`folder-${String(index).padStart(2, '0')}`, dir({})]))

/**
 * The tree over `/w` holding `children`, showing what `selected` names, as
 * the File view passes it the file it shows; a folder named in `held` lists
 * only once let go.
 */
function mountTree(children: Parameters<typeof dir>[0], held: string[] = []) {
  const gates = new Map(held.map((path) => [path, deferred()]))
  const reads = memoryFileReads({ platform: 'linux', home: '/w', root: dir({ w: dir(children) }) })
  const source = keptSource({
    ...reads,
    list: async (path) => {
      await gates.get(path)?.promise
      return reads.list(path)
    },
  }, { files: new HostFiles() })
  const selected = ref<string | null>(null)
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp({ render: () => h(FileTree, { source, root: '/w', selected: selected.value }) })
  app.mount(host)
  teardown.push(() => app.unmount())
  const row = (name: string) => [...host.querySelectorAll<HTMLElement>('[role=treeitem]')].find((el) => el.textContent?.trim() === name)
  return {
    selected,
    row,
    let: (path: string) => gates.get(path)!.resolve(),
    selectedRows: () => [...host.querySelectorAll('[role=tree] > [aria-selected=true]')].map((el) => el.textContent?.trim()),
    scrollTop: () => host.querySelector<HTMLElement>('.scroll-area-viewport')!.scrollTop,
    /** Lets the listings land and the tree render them, until `done` holds. */
    async until(done: () => boolean) {
      for (let turn = 0; turn < 50 && !done(); turn++) {
        await nextTick()
        await new Promise((resolve) => setTimeout(resolve, 0))
      }
      expect(done()).toBe(true)
    },
  }
}

/** Whether a row lies within the tree's view, by the stand-in layout. */
function inView(row: HTMLElement, scrollTop: number): boolean {
  const top = row.offsetTop - scrollTop
  return top >= FIRST_ROW_PX && top + TREE_ROW_PITCH_PX <= VIEW_PX + 1
}

test('a deep file the view shows unfolds the folders above it, is selected and scrolled into view; one in view leaves the tree still', async () => {
  const tree = mountTree({ ...many, 'top.ts': textFile('', at), zeta: dir({ inner: dir({ 'deep.ts': textFile('', at) }) }) })
  tree.selected.value = '/w/folder-01'
  await tree.until(() => tree.selectedRows().includes('folder-01'))
  await nextTick()
  expect(tree.scrollTop()).toBe(0)

  tree.selected.value = '/w/zeta/inner/deep.ts'
  await tree.until(() => tree.selectedRows().includes('deep.ts'))
  await nextTick()
  expect(tree.row('zeta')!.getAttribute('aria-expanded')).toBe('true')
  expect(tree.row('inner')!.getAttribute('aria-expanded')).toBe('true')
  expect(tree.selectedRows()).toEqual(['deep.ts'])
  expect(inView(tree.row('deep.ts')!, tree.scrollTop())).toBe(true)
})

test('a file shown while another’s folders still list wins: the first’s deeper folders stay closed', async () => {
  const tree = mountTree({
    ...many,
    one: dir({ deeper: dir({ 'a.ts': textFile('', at) }) }),
    two: dir({ 'b.ts': textFile('', at) }),
  }, ['/w/one'])
  tree.selected.value = '/w/one/deeper/a.ts'
  await tree.until(() => tree.row('one')?.getAttribute('aria-expanded') === 'true')

  tree.selected.value = '/w/two/b.ts'
  await tree.until(() => tree.selectedRows().includes('b.ts'))
  await nextTick()
  expect(inView(tree.row('b.ts')!, tree.scrollTop())).toBe(true)
  // The first file's folder lists only now, too late to be followed.
  tree.let('/w/one')
  await tree.until(() => tree.row('deeper') !== undefined)
  await nextTick()
  expect(tree.row('deeper')!.getAttribute('aria-expanded')).toBe('false')
  expect(tree.row('a.ts')).toBeUndefined()
  expect(tree.selectedRows()).toEqual(['b.ts'])
})

test('a file outside the tree’s root clears the selection', async () => {
  const tree = mountTree({ 'notes.md': textFile('', at) })
  tree.selected.value = '/w/notes.md'
  await tree.until(() => tree.selectedRows().includes('notes.md'))

  tree.selected.value = '/elsewhere/notes.md'
  await nextTick()
  await nextTick()
  expect(tree.selectedRows()).toEqual([])
})
