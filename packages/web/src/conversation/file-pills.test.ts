import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'
import type { Block, ChangedFile, EditedFile, ModelSelection } from '@demicodes/protocol'

// A conversation outside a project shows no changed-file pills, under a
// call or under a folded run; one in a project shows both
// (`edit-tracking.md` § What the conversation shows). The rows are the
// product's runs on a page of happy-dom, under what the conversation
// session provides from what the conversation page tells it; the message
// list around them lays out nothing in happy-dom. A few milliseconds,
// besides the start of the page.

GlobalRegistrator.register({ url: 'http://127.0.0.1:3271/chat' })
await import('../__tests__/vue-components')
const { createApp, h, nextTick, ref } = await import('vue')
const { provideBlobUrl } = await import('@demicodes/web-ui/agent/media-source')
const { provideMediaViewer } = await import('@demicodes/web-ui/files/media-viewer')
const { provideFilePills } = await import('@demicodes/web-ui/agent/edit-selection')
const { groupWork } = await import('@demicodes/web-ui/agent/work-groups')
const { default: WorkGroupBlock } = await import('@demicodes/web-ui/agent/blocks/WorkGroupBlock.vue')

const createdAt = '2026-10-15T12:00:00.000Z'
const model: ModelSelection = {
  providerId: 'stub',
  model: { id: 'stub', name: 'Stub', contextWindow: 1000, outputLimit: null, thinking: [], acceptedExtensions: [] },
  thinking: null,
  serviceTierId: null,
}

/** A file a shell call wrote, as its view and its run's list store it. */
const own = (path: string): EditedFile => ({ path, kind: 'modified', added: 1, removed: 1, edits: [] })
const listed = (path: string): ChangedFile => ({ path, kind: 'modified', lines: { added: 1, removed: 1 }, edits: [] })

/** A shell call of the run `run` that wrote `path`, its run having written `runPaths`. */
function shellCall(id: string, run: string, path: string, runPaths: string[]): Block {
  return {
    type: 'tool_call', id, run, createdAt, model, toolUseId: `use-${id}`, toolName: 'shell',
    input: JSON.stringify({ script: 'true', description: `Write ${path}` }), status: 'completed', endedAt: createdAt, output: [],
    view: {
      kind: 'shell', status: 'exited', commandId: id, exitCode: 0, runningMs: 10, idleMs: 0, chunks: [],
      viewTruncated: false, files: [own(path)], runFiles: runPaths.map(listed),
    },
  }
}

const user = (id: string): Block => ({ type: 'user', id, turnId: id, createdAt, model, content: [{ type: 'text', text: 'Fix it.' }], preamble: null })
const reply = (id: string): Block => ({ type: 'text', id, createdAt, model, text: 'Done.' })

// A lone call, then a run of two calls, which folds into one row with the
// pills of both its files.
const blocks: Block[] = [
  user('u1'), shellCall('c1', 'c1', '/w/lone.ts', ['/w/lone.ts']), reply('t1'),
  user('u2'), shellCall('c2', 'c2', '/w/first.ts', ['/w/first.ts']), shellCall('c3', 'c2', '/w/second.ts', ['/w/first.ts', '/w/second.ts']), reply('t2'),
]

const pages: (() => void)[] = []
afterEach(() => {
  pages.splice(0).forEach((unmount) => unmount())
})

test('a conversation outside a project shows no file pills, and one moved into a project shows them', async () => {
  const inProject = ref(false)
  const element = document.createElement('div')
  document.body.append(element)
  const app = createApp({
    setup() {
      provideBlobUrl((blob) => `/blobs/${blob}`)
      provideMediaViewer()
      // As the conversation session provides it from what its page says.
      provideFilePills(() => inProject.value)
      // The transcript's rows, as its message list draws them: the lone
      // call is its run's row, which shows the call's own.
      const runs = groupWork(blocks, false).filter((row) => row.type === 'work_group')
      return () => runs.map((group) => h(WorkGroupBlock, { key: group.id, group }))
    },
  })
  app.mount(element)
  pages.push(() => app.unmount())
  await nextTick()
  const pills = () => [...element.querySelectorAll('.file-pill')].map((pill) => pill.textContent?.trim())

  // The calls show, their pills do not.
  expect(element.textContent).toContain('Write /w/lone.ts')
  expect(pills()).toEqual([])

  inProject.value = true
  await nextTick()
  expect(pills()).toEqual(['lone.ts+1−1', 'first.ts+1−1', 'second.ts+1−1'])
})
