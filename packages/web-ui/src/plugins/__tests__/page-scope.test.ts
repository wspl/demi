import { expect, test } from 'bun:test'
import { createRenderer, defineComponent, h, nextTick, ref } from 'vue'
import { z } from 'zod'
import { createOverlayStore } from '../../overlay/overlayStore'
import { PageScope, definePage, providePageHost, usePage, type PageHost } from '../page'

// Moving between two plugins' settings sections shows the new page's
// component with the new page's context, never the previous page's. A page
// of happy-dom and one Vue app; a few milliseconds.

/** A renderer over plain objects: the test needs the components, not a page. */
interface Node {
  parent: Node | null
  children: Node[]
}
const node = (): Node => ({ parent: null, children: [] })
const { createApp } = createRenderer<Node, Node>({
  patchProp: () => {},
  insert(child, parent, anchor) {
    child.parent = parent
    parent.children.splice(anchor ? parent.children.indexOf(anchor) : parent.children.length, 0, child)
  },
  remove(child) {
    child.parent?.children.splice(child.parent.children.indexOf(child), 1)
  },
  createElement: node,
  createText: node,
  createComment: node,
  setText: () => {},
  setElementText: () => {},
  parentNode: (child) => child.parent,
  nextSibling: (child) => child.parent?.children[child.parent.children.indexOf(child) + 1] ?? null,
})

/** A host whose user state is each plugin's own, and whose every other part refuses to be used. */
function host(): PageHost {
  const unused = () => {
    throw new Error('the test does not use this part of the host')
  }
  return {
    userState: (plugin) => ({ plugin }),
    followState: unused,
    call: unused,
    stream: unused,
    installed: unused,
    hostStarting: unused,
    files: unused,
    intents: { open: unused, canOpen: unused },
    panel: { tabs: unused, add: unused, select: unused },
    openSettings: unused,
    openLink: unused,
    overlays: createOverlayStore(),
  }
}

test('a page shown after another one reads its own plugin’s state', async () => {
  const read: string[] = []
  // A component of a page that reads its plugin's state, as each plugin's settings do.
  const reader = defineComponent({
    setup() {
      const state = usePage().plugin.state(z.object({ plugin: z.string() }))
      return () => {
        read.push(state.value?.plugin ?? 'nothing')
        return h('span')
      }
    },
  })
  const skills = definePage({ plugin: 'skills' })
  const web = definePage({ plugin: 'web' })
  const shown = ref(skills)
  const root = node()
  const app = createApp({
    setup() {
      providePageHost(host())
      return () => h(PageScope, { page: shown.value, component: reader })
    },
  })
  app.mount(root)
  expect(read.at(-1)).toBe('skills')

  shown.value = web
  await nextTick()
  expect(read.at(-1)).toBe('web')

  shown.value = skills
  await nextTick()
  expect(read.at(-1)).toBe('skills')
  app.unmount()
})
