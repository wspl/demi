import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A live child that compacted shows a window that starts inside a request,
// after its `user` block. Its edit pills found no request without that block
// and were dead spans; each block now names its request. The pills of a call
// in such a window, in the transcript context the message list gives its
// rows, on a page of happy-dom; a few ms besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h } = await import('vue')
const { default: ShellEditPills } = await import('../blocks/ShellEditPills.vue')
const { provideEditSelection, provideTranscript } = await import('../edit-selection')
const { storedRequests } = await import('../../files/request-changes')
const { waitFor } = await import('@demicodes/utils')
const { createdAt, model } = await import('./agent-harness')
type Block = import('@demicodes/protocol').Block
type ToolCallBlock = import('../block-types').ToolCallBlock

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

const path = '/work/login.ts'
const edit = { call: 'c1', path, segment: 0, copies: { original: '1'.repeat(64), modified: '2'.repeat(64) } }
/** The request's latest call, which edits `login.ts`. */
const call: ToolCallBlock = {
  type: 'tool_call',
  id: 'call-1',
  run: 'call-1',
  request: 'u1',
  createdAt,
  model,
  toolUseId: 'use-1',
  toolName: 'shell',
  input: JSON.stringify({ script: 'true', description: 'Tighten the check' }),
  status: 'completed',
  endedAt: createdAt,
  output: [],
  view: {
    kind: 'shell',
    status: 'exited',
    commandId: 'c1',
    exitCode: 0,
    runningMs: 10,
    idleMs: 0,
    chunks: [],
    viewTruncated: false,
    files: [{ path, kind: 'modified', added: 1, removed: 1, edits: [{ copies: edit.copies }] }],
    runFiles: [{ path, kind: 'modified', lines: { added: 1, removed: 1 }, edits: [edit] }],
    requestFiles: [{ path, kind: 'modified', lines: { added: 1, removed: 1 }, edits: [edit] }],
  },
}

test('an edit pill of a child whose window starts inside a request opens its edit', async () => {
  // The child's window: the call and the reply after it, not the `user` block before.
  const window: Block[] = [call, { type: 'text', id: 'reply', request: 'u1', createdAt, model, text: 'Done.' }]
  const picked: unknown[] = []
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp(defineComponent({
    setup() {
      provideEditSelection(() => (selection) => picked.push(selection))
      provideTranscript({
        node: 'child',
        requests: () => storedRequests(window),
        directory: () => '/work',
        pictures: () => [],
      })
      return () => h(ShellEditPills, { block: call })
    },
  }))
  app.mount(root)
  unmount = () => app.unmount()
  let pill: HTMLElement | null = null
  await waitFor(() => (pill = root.querySelector<HTMLElement>('.file-pill')) !== null, () => 'no pill')
  expect(pill!.tagName).toBe('BUTTON')
  pill!.click()

  expect(picked).toEqual([{ node: 'child', request: 'u1', file: path, edit: { call: 'c1', path, segment: 0 }, root: '/work' }])
})
