import { createPinia } from 'pinia'
import { afterEach, expect, test } from 'bun:test'
import { createRenderer, defineComponent, h, nextTick, ref, type App } from 'vue'
import { z } from 'zod'
import { createOverlayStore } from '../../overlay/overlayStore'
import { useOperationControl } from '../../ui/operations'
import { PageScope, definePage, providePageHost, usePage, type PageHost } from '../page'

// A plugin's call is an operation of the page: the Skills page's switch of a skill goes on with its
// call when the page goes, shows it working when drawn again, and starts nothing while it runs. The
// switch is drawn by Vue over plain objects through the page context and the press Switch uses; a
// few milliseconds.

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

/** The calls the plugin received, each answered when the test chooses. */
interface Received {
  method: string
  params: object
  answer: () => void
}

/** A host whose plugin calls wait for the test, and whose every other part refuses to be used. */
function host(received: Received[]): PageHost {
  const unused = () => {
    throw new Error('the test does not use this part of the host')
  }
  return {
    userState: () => undefined,
    followState: unused,
    call: (_plugin, method, params) => {
      const { promise, resolve } = Promise.withResolvers<unknown>()
      received.push({ method, params, answer: () => resolve(null) })
      return promise
    },
    stream: unused,
    installed: unused,
    hostStarting: unused,
    files: unused,
    intents: { open: unused, canOpen: unused },
    panel: { tabs: unused, select: unused },
    openSettings: unused,
    openLink: unused,
    overlays: createOverlayStore(),
    pinia: createPinia(),
  }
}

/** The skill switch the page draws last: whether it shows its call working, and its flip. */
interface DrawnSwitch {
  working: () => boolean
  flip: () => void
}

let apps: App[] = []
afterEach(() => {
  for (const app of apps) {
    app.unmount()
  }
  apps = []
})

/** Lets an answered call end its operation. */
async function settle(): Promise<void> {
  for (let turn = 0; turn < 5; turn++) {
    await Promise.resolve()
  }
  await nextTick()
}

test("a skill switch's call goes on when its page goes, and the switch drawn again shows it and starts nothing", async () => {
  const received: Received[] = []
  const drawn: DrawnSwitch[] = []
  // The switch of skill `review` in source `src_1`, as the Skills page binds it.
  const skillSwitch = defineComponent({
    setup() {
      const page = usePage()
      const { running, press } = useOperationControl(() => page.operation('set_enabled', 'src_1/review'), false)
      drawn.push({
        working: () => running.value,
        flip: () => press(() => page.plugin.call('set_enabled', { source: 'src_1', skill: 'review', enabled: true }, z.null())),
      })
      return () => h('span')
    },
  })
  const skills = definePage({ plugin: 'skills' })
  const shown = ref(true)
  const app = createApp({
    setup() {
      providePageHost(host(received))
      return () => (shown.value ? h(PageScope, { page: skills, component: skillSwitch }) : null)
    },
  })
  app.mount(node())
  apps.push(app)

  drawn.at(-1)!.flip()
  expect(received.map((call) => call.method)).toEqual(['set_enabled'])
  expect(drawn.at(-1)!.working()).toBe(true)

  // Settings closes and opens again: the call goes on, and the new switch shows it.
  shown.value = false
  await nextTick()
  shown.value = true
  await nextTick()
  expect(drawn).toHaveLength(2)
  expect(drawn[1]!.working()).toBe(true)

  // A second flip while it runs starts nothing.
  drawn[1]!.flip()
  expect(received).toHaveLength(1)

  received[0]!.answer()
  await settle()
  expect(drawn[1]!.working()).toBe(false)
  drawn[1]!.flip()
  expect(received).toHaveLength(2)
  received[1]!.answer()
  await settle()
})
