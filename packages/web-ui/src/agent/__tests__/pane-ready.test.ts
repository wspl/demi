import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, jest, test } from 'bun:test'

// A pane of the work panel changes whole (docs/decisions/0038-a-pane-changes-whole.md):
// a click makes the new tab's view the shown one at once, its area stays empty
// while its parts read, draws when the last is in, and says what it reads past
// 300 ms. One Vue app on a happy-dom page, a few ms.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, reactive, shallowRef } = await import('vue')
const { z } = await import('zod')
const { default: WorkPanel } = await import('../WorkPanel.vue')
const { default: TreeFrame } = await import('../../files/TreeFrame.vue')
const { usePaneReady, PANE_WAIT_MS } = await import('../pane-ready')
type PanelTabKind<Data = unknown> = import('../panel-kinds/kind').PanelTabKind<Data>

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
  jest.useRealTimers()
})

const Empty = defineComponent({ setup: () => () => null })

/** What each tab's reads have brought: its title is in once it is true. */
const arrived = reactive<Record<string, boolean>>({})

/** A view as the File view is built: a header row, and under it an area that waits for its parts. */
const View = defineComponent({
  props: { tabId: { type: String, required: true }, data: { type: Object, required: true }, shown: Boolean },
  setup(props) {
    const title = (props.data as { title: string }).title
    usePaneReady(() => arrived[title] === true)
    return () => h(TreeFrame, { tree: 'files' }, {
      header: () => h('span', { 'data-address': title }, title),
      tree: () => h('div', { 'data-tree': title }, `tree of ${title}`),
      default: () => h('div', { 'data-body': title }, `body of ${title}`),
    })
  },
})

const kind: PanelTabKind<{ title: string }> = {
  kind: 'page',
  schema: z.object({ title: z.string() }),
  title: (data) => data.title,
  mark: Empty,
  content: View as never,
}

function mountPanel(titles: string[], selected: { value: string }) {
  const tabs = titles.map((title) => ({ id: title, kind: 'page', data: { title } }))
  const root = document.createElement('div')
  document.body.append(root)
  const kinds = [kind]
  const app = createApp({
    render: () => h(WorkPanel, { panel: { history: [selected.value], tabs }, pinned: {}, kinds }),
  })
  app.mount(root)
  unmount = () => app.unmount()
  return root
}

/** The box of a view's tree, the one that is empty until the pane has been whole once. */
function tree(root: HTMLElement, title: string): HTMLElement {
  return root.querySelector<HTMLElement>(`[data-tree="${title}"]`)!.closest<HTMLElement>('.border-l')!
}

/** The area under a view's header, the one the pane empties. */
function area(root: HTMLElement, title: string): HTMLElement {
  return root.querySelector<HTMLElement>(`[data-body="${title}"]`)!.closest<HTMLElement>('.relative.isolate')!
}

test('a pane\'s area stays empty at its size while its parts read, and draws when the last is in', async () => {
  jest.useFakeTimers()
  for (const key of Object.keys(arrived))
    delete arrived[key]
  const selected = shallowRef('a.ts')
  const sel = { get value() { return selected.value } }
  const root = mountPanel(['a.ts'], sel)
  await nextTick()

  // The address is the view's from the first frame; the area under it is empty.
  expect(root.querySelector('[data-address="a.ts"]')).not.toBeNull()
  expect(area(root, 'a.ts').className).toContain('invisible')
  expect(tree(root, 'a.ts').className).toContain('invisible')
  expect(root.querySelector('[data-pane-reading]')).toBeNull()

  // Past 300 ms it says what it reads.
  jest.advanceTimersByTime(PANE_WAIT_MS)
  await nextTick()
  expect(root.querySelector('[data-pane-reading]')?.textContent).toBe('Reading a.ts')

  // The last part arrives: the area draws whole, and the sentence goes.
  arrived['a.ts'] = true
  await nextTick()
  expect(area(root, 'a.ts').className).not.toContain('invisible')
  expect(tree(root, 'a.ts').className).not.toContain('invisible')
  expect(root.querySelector('[data-pane-reading]')).toBeNull()

  // The same pane reads again, as when another file is picked: its tree stays and the view is empty.
  arrived['a.ts'] = false
  await nextTick()
  expect(area(root, 'a.ts').className).toContain('invisible')
  expect(tree(root, 'a.ts').className).not.toContain('invisible')
})

test('a view that is in before 300 ms never says it reads', async () => {
  jest.useFakeTimers()
  for (const key of Object.keys(arrived))
    delete arrived[key]
  arrived['a.ts'] = false
  const selected = shallowRef('a.ts')
  const root = mountPanel(['a.ts', 'b.ts'], { get value() { return selected.value } })
  await nextTick()

  jest.advanceTimersByTime(PANE_WAIT_MS - 1)
  arrived['a.ts'] = true
  await nextTick()
  jest.advanceTimersByTime(PANE_WAIT_MS * 2)
  await nextTick()
  expect(root.querySelector('[data-pane-reading]')).toBeNull()
  expect(area(root, 'a.ts').className).not.toContain('invisible')
})

test('switching to a view not read yet shows it at once, empty, and switching back shows the one read as it was', async () => {
  jest.useFakeTimers()
  for (const key of Object.keys(arrived))
    delete arrived[key]
  arrived['a.ts'] = true
  const selected = shallowRef('a.ts')
  const root = mountPanel(['a.ts', 'b.ts'], { get value() { return selected.value } })
  await nextTick()
  expect(area(root, 'a.ts').className).not.toContain('invisible')

  // The click: the strip and the address are b.ts's in the same frame, its area is empty,
  // and nothing of a.ts is left on show.
  selected.value = 'b.ts'
  await nextTick()
  expect(root.querySelector('[data-address="b.ts"]')).not.toBeNull()
  expect(area(root, 'b.ts').className).toContain('invisible')
  const hidden = root.querySelector<HTMLElement>('[data-body="a.ts"]')!.closest<HTMLElement>('[style]')!
  expect(hidden.style.display).toBe('none')
  expect(root.querySelector('[data-pane-reading]')).toBeNull()

  // It waited past 300 ms since the click: it says what it reads, and then draws whole.
  jest.advanceTimersByTime(PANE_WAIT_MS)
  await nextTick()
  expect(root.querySelector('[data-pane-reading]')?.textContent).toBe('Reading b.ts')
  arrived['b.ts'] = true
  await nextTick()
  expect(area(root, 'b.ts').className).not.toContain('invisible')

  // Back to the one read: it shows in the same frame, with no wait.
  selected.value = 'a.ts'
  await nextTick()
  expect(area(root, 'a.ts').className).not.toContain('invisible')
  expect(root.querySelector('[data-pane-reading]')).toBeNull()
})
