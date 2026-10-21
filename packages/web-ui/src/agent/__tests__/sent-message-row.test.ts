import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A message the user sends keeps one row from the send to the message the turn
// writes: the server may tell the turn's starting message in between, held over
// a painted frame or gone within one, and the row neither rolls (`RowRoll`) nor
// shows the bubble twice. Before, the starting message was a row of another
// name and face, so the bubble rolled out and back in. The list component on a
// happy-dom page; a few ms besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { h, nextTick, reactive } = await import('vue')
const { default: AgentMessageList } = await import('../AgentMessageList.vue')
const { drawn, settled } = await import('./drawn-list')
const { createdAt, model, userBlock } = await import('./agent-harness')
type Block = import('@demicodes/protocol').Block
type QueuedMessage = import('@demicodes/protocol').QueuedMessage
type SessionPhase = import('@demicodes/protocol').SessionPhase
type PendingSubmissionState = import('../types').PendingSubmissionState

const cleanups: (() => void)[] = []
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) {
    cleanup()
  }
})

const WORDS = 'Rename the folder,\nthen list what it holds.'

interface ListState {
  blocks: Block[]
  phase: SessionPhase
  starting: QueuedMessage | null
  pendingSubmissions: PendingSubmissionState[]
}

/**
 * The keys of the rows that show the sent message, and the copies of it that
 * a roll moves out of its row.
 */
function messageRows(host: HTMLElement): { keys: string[]; leaving: number } {
  const rows = [...host.querySelectorAll<HTMLElement>('[data-row-key]')]
    .filter((row) => row.textContent?.includes('Rename the folder'))
  return {
    keys: rows.map((row) => row.dataset.rowKey!),
    leaving: rows.reduce((count, row) => count + row.querySelectorAll('[inert]').length, 0),
  }
}

test.each([
  { name: 'held across a painted frame', paint: settled },
  { name: 'delivered within one frame', paint: nextTick },
])('a sent message whose starting is $name keeps one stationary row from its send to the message written', async ({ paint }) => {
  const state = reactive<ListState>({
    blocks: [userBlock('u-1', 'm-1', 'Sort the downloads.'), { type: 'text', id: 'a-1', createdAt, model, text: 'Sorted.' }],
    phase: 'idle',
    starting: null,
    pendingSubmissions: [],
  })
  const view = drawn(() => h(AgentMessageList, {
    conversationId: 'c-1',
    blocks: state.blocks,
    phase: state.phase,
    starting: state.starting,
    pendingSubmissions: state.pendingSubmissions,
    pendingSteers: [],
    queue: [],
    load: 'ready',
    bottomOffset: 0,
    persistedScrollState: undefined,
  }))
  cleanups.push(view.unmount)
  await settled()

  const seen: { name: string; keys: string[]; leaving: number }[] = []
  async function step(name: string, change: () => void, after: () => Promise<void>): Promise<void> {
    change()
    await after()
    const { keys, leaving } = messageRows(view.host)
    seen.push({ name, keys, leaving })
  }
  const message = { id: 'm-2', content: [{ type: 'text' as const, text: WORDS }] }
  await step('sent', () => {
    state.pendingSubmissions = [{ id: 'm-2', way: 'send', text: WORDS, attachments: [], error: null }]
  }, settled)
  await step('starting', () => {
    state.phase = 'running'
    state.starting = message
  }, paint)
  await step('confirmed', () => {
    state.pendingSubmissions = []
  }, paint)
  await step('written', () => {
    state.blocks = [...state.blocks, { ...userBlock('u-2', 'm-2', WORDS), content: message.content }]
    state.starting = null
  }, settled)

  const row = seen[0]!.keys
  expect(row).toHaveLength(1)
  expect(seen).toEqual(['sent', 'starting', 'confirmed', 'written'].map((name) => ({ name, keys: row, leaving: 0 })))
})
