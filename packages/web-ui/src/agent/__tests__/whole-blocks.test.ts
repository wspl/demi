import { expect, test } from 'bun:test'
import { createRenderer, defineComponent, h, nextTick, ref } from 'vue'
import { EMPTY_TRANSCRIPT, heldBlocks, resetTranscript, type HeldBlock } from '@demicodes/conversation-client'
import { provideBlockReader, provideBlockScope, useWholeBlock, type ShownBlock } from '../whole-blocks'
import { createdAt, model } from './agent-harness'

// Cost: one Vue app over plain objects, a few milliseconds.

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

const lightThinking = (id: string): HeldBlock => ({ type: 'thinking', id, createdAt, model, text: '', signature: null, textLength: 12, light: true })
const joined = (...parts: HeldBlock[]): ShownBlock => ({ ...parts[0]!, parts })

// Thinking blocks next to each other are one step, which opens to read each
// of its light blocks whole. The next page of history may bring another such
// block into the step while it is open; before, only the blocks there when
// it opened were read, so the step showed the first text alone until it was
// folded and opened again.
test('a light block that joins an open row is read whole too', async () => {
  const reads: string[] = []
  const shown = ref<ShownBlock>(joined(lightThinking('k1')))
  const open = ref(false)
  const row = defineComponent({
    setup() {
      useWholeBlock(() => open.value)
      return () => null
    },
  })
  const app = createApp(defineComponent({
    setup() {
      provideBlockReader(() => async (_node, id) => {
        reads.push(id)
      })
      provideBlockScope(() => shown.value)
      return () => h(row)
    },
  }))
  app.mount(node())
  await nextTick()
  expect(reads).toEqual([])
  open.value = true
  await nextTick()
  expect(reads).toEqual(['k1'])
  // The next page brings k2 next to k1 while k1 is still on its way.
  shown.value = joined(lightThinking('k1'), lightThinking('k2'))
  await nextTick()
  expect(reads).toEqual(['k1', 'k2'])
  app.unmount()
})

// A window opened again, or a page that reconnects, receives its blocks in a
// reset, light as a page's are: a thinking without its text, a call without
// its output. Before, the reset's blocks were not marked light, so opening
// one showed it empty and read nothing.
test('a block that came in a reset is read whole when its row opens', async () => {
  const { light: _, ...sent } = lightThinking('k1')
  const reset = resetTranscript(EMPTY_TRANSCRIPT, { start: 0, length: 2, blocks: [sent, { ...sent, id: 'k2' }] })
  const reads: string[] = []
  const open = ref(false)
  const row = defineComponent({
    setup() {
      useWholeBlock(() => open.value)
      return () => null
    },
  })
  const app = createApp(defineComponent({
    setup() {
      provideBlockReader(() => async (_node, id) => {
        reads.push(id)
      })
      provideBlockScope(() => joined(heldBlocks(reset)[0]!))
      return () => h(row)
    },
  }))
  app.mount(node())
  open.value = true
  await nextTick()
  expect(reads).toEqual(['k1'])
  app.unmount()
})
