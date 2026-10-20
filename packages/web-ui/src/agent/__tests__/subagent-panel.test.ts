import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// The Agents window over a child whose transcript the page does not hold: it
// says it loads while the read runs, and a failed read says so with Retry,
// never an empty transcript. Production showed a blank panel while a child's
// read took a minute, and nothing at all when it failed. A Vue app on a
// happy-dom page; a few ms besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, reactive } = await import('vue')
const { default: SubagentPanel } = await import('../SubagentPanel.vue')
const { default: DockWindowArea } = await import('../DockWindowArea.vue')
const { provideBlobUrl } = await import('../media-source')
const { provideMediaViewer } = await import('../../files/media-viewer')
const { EMPTY_TRANSCRIPT, heldTranscript } = await import('@demicodes/conversation-client')
const { deferred, waitFor } = await import('@demicodes/utils')
const { createdAt, userBlock } = await import('./agent-harness')
type SubagentRecord = import('../subagents').SubagentRecord
type Deferred = import('@demicodes/utils').Deferred<void>

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

/** A child the page holds none of: live, or `completed` once it ended. */
function child(phase: 'running' | 'completed'): SubagentRecord {
  return {
    id: 'child',
    number: 1,
    name: 'Read the notes',
    phase,
    onPath: true,
    profile: null,
    model: { providerId: 'anthropic', provider: 'Anthropic', model: 'Claude Opus 5.5', effort: 'high', fast: false },
    round: 1,
    forkedFrom: null,
    startedAt: createdAt,
    history: EMPTY_TRANSCRIPT,
    sessionPhase: phase === 'running' ? 'running' : 'idle',
    pendingCalls: [],
    waitingSince: null,
    failures: {},
    contextUsage: null,
    cwd: '/work',
  }
}

function text(): string {
  return document.body.textContent ?? ''
}

/** The failure's Retry, once the window shows it. */
function retryButton(): HTMLElement | undefined {
  return [...document.querySelectorAll<HTMLElement>('[role="button"]')].find((button) => button.textContent?.trim() === 'Retry')
}

/** Mounts the window over `agents`, showing the first, with `readLatest` as its reader. */
function mountPanel(agents: SubagentRecord[], readLatest: (id: string) => Promise<void>, streamed = true): void {
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp(defineComponent({
    setup: () => {
      provideBlobUrl((source) => `blob:${source}`)
      provideMediaViewer()
      return () => h(DockWindowArea, { area: 600 }, () => [
        h(SubagentPanel, { agents, open: true, activeId: agents[0]!.id, readLatest, streamed }),
      ])
    },
  }))
  app.mount(host)
  unmount = () => app.unmount()
}

test('a finished child the window holds none of shows loading, a failed read shows Retry, and Retry reads it again', async () => {
  const reads: Deferred[] = []
  const agents = reactive([child('completed')])
  mountPanel(agents, () => {
    const read = deferred()
    reads.push(read)
    return read.promise
  })
  await waitFor(() => reads.length === 1 && text().includes('Loading agent…'), () => `not loading: ${text()}`)
  reads[0]!.reject(new Error('The server took too long to answer.'))
  await waitFor(() => retryButton() !== undefined, () => `no Retry: ${text()}`)
  const failed = text()
  retryButton()!.click()
  await waitFor(() => reads.length === 2 && text().includes('Loading agent…'), () => `Retry read nothing: ${text()}`)
  agents[0]!.history = heldTranscript(1, [{ start: 0, blocks: [userBlock('u1', 't1', 'Read notes.md')] }])
  reads[1]!.resolve()
  await waitFor(() => !text().includes('Loading agent…'), () => `still loading: ${text()}`)
  const shown = text()

  expect(failed).toContain('Couldn’t load this agent.')
  expect(failed).toContain('The server took too long to answer.')
  expect(shown).not.toContain('Couldn’t load this agent.')
})

// A live child's transcript comes with the socket's reset once the page
// shows it (`crates/agent-server/src/server/tree.rs`): reading it over HTTP as well read a
// large child's page twice. It loads until the reset arrives.
test('showing a live child reads nothing and loads until its transcript arrives', async () => {
  const reads: string[] = []
  const agents = reactive([child('running')])
  mountPanel(agents, async (id) => {
    reads.push(id)
  })
  await waitFor(() => text().includes('Loading agent…'), () => `not loading: ${text()}`)
  agents[0]!.history = heldTranscript(1, [{ start: 0, blocks: [userBlock('u1', 't1', 'Read notes.md')] }])
  await waitFor(() => !text().includes('Loading agent…'), () => `still loading: ${text()}`)

  expect(reads).toEqual([])
})

// A page that opens no session, as for a conversation whose model is gone,
// receives no child's transcript, though the record calls the child
// running: the window read nothing and said it loads for ever. It reads
// the child's latest page as it reads a finished one's.
test('a live child on a page with no session is read', async () => {
  const reads: string[] = []
  const agents = reactive([child('running')])
  mountPanel(agents, async (id) => {
    reads.push(id)
  }, false)
  await waitFor(() => reads.length > 0, () => 'nothing read')

  expect(reads).toEqual(['child'])
})
