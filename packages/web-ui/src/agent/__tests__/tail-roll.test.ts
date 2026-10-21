import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// Only rows drawn with FunctionalBlock roll (`RowRoll`) into one another: a
// step that follows the Requesting row rolls over it, and so does Requesting
// over the Stopped row on Continue, while the answer's text and an error
// appear in Requesting's place without a roll. Before, every row took the
// place of the row it followed and rolled over it, so the answer rolled up
// over Requesting. The list component on a happy-dom page; a few ms besides
// the page's start, and each roll, which runs to its end (about 0.5 s).

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { h, nextTick, reactive } = await import('vue')
const { default: AgentMessageList } = await import('../AgentMessageList.vue')
const { drawn, settled, tailRow } = await import('./drawn-list')
const { createdAt, model, userBlock } = await import('./agent-harness')
const { t } = await import('../locales/text')
type Block = import('@demicodes/protocol').Block
type AgentActivity = import('../agent-activity').AgentActivity
type PendingAction = import('../activity-slot').PendingAction
type SessionPhase = import('@demicodes/protocol').SessionPhase

interface ListState {
  blocks: Block[]
  phase: SessionPhase
  activity: AgentActivity
  pendingAction: PendingAction
}

const cleanups: (() => void)[] = []
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) {
    cleanup()
  }
})

const WAITING: AgentActivity = { pendingCalls: [], waitingSince: Date.now(), actionSent: false }
const ANSWERED: AgentActivity = { pendingCalls: [], waitingSince: null, actionSent: false }

const answer: Block = { type: 'text', id: 'a-1', createdAt, model, text: 'The folder holds three files.' }
const step: Block = {
  type: 'tool_call',
  id: 'c-1',
  createdAt,
  model,
  toolUseId: 'c-1-use',
  toolName: 'shell',
  status: 'executing',
  input: JSON.stringify({ script: 'ls', description: 'List the folder' }),
  endedAt: null,
  output: [],
  view: null,
  run: 'c-1',
} as Block

/**
 * The list drawn from `before`, then changed to `after`: for each frame from
 * the change on, the words of the rows that hold a copy a roll moves out, and
 * what the tail row says once the change is drawn.
 */
async function rolls(before: ListState, after: Partial<ListState>): Promise<{ frames: string[][], tail: string | null }> {
  const state = reactive<ListState>(before)
  const view = drawn(() => h(AgentMessageList, {
    conversationId: 'c-1',
    blocks: state.blocks,
    phase: state.phase,
    activity: state.activity,
    pendingAction: state.pendingAction,
    pendingSteers: [],
    queue: [],
    load: 'ready',
    bottomOffset: 0,
    persistedScrollState: undefined,
  }))
  cleanups.push(view.unmount)
  await settled()

  Object.assign(state, after)
  const frames: string[][] = []
  for (const paint of [nextTick, nextTick, settled]) {
    await paint()
    frames.push([...view.host.querySelectorAll<HTMLElement>('[data-row-key]')]
      .filter((row) => row.querySelector('[data-roll-leaving]'))
      .map((row) => row.textContent?.trim() ?? ''))
  }
  // A roll cut short by the unmount would reject its animations' `finished`,
  // which happy-dom, unlike a browser, reports as unhandled: the roll ends
  // first, taking its copy with it.
  while (view.host.querySelector('[data-roll-leaving]')) {
    await settled()
  }
  return { frames, tail: tailRow(view.host) }
}

const asked: Block[] = [userBlock('u-1', 'm-1', 'What does the folder hold?')]
const requesting = (): ListState => ({ blocks: asked, phase: 'running', activity: WAITING, pendingAction: null })

test('a share\'s note of steps not shared is drawn as a functional row', async () => {
  const { drawnAsFunctionalBlock } = await import('../row-key')
  expect(drawnAsFunctionalBlock({ type: 'steps_not_shared', id: 'steps:1', count: 3, after: null })).toBe(true)
})

test('the answer\'s text takes the Requesting row\'s place without a roll', async () => {
  expect(await rolls(requesting(), { blocks: [...asked, answer], activity: ANSWERED })).toEqual({ frames: [[], [], []], tail: null })
})

test('a step takes the Requesting row\'s place by rolling over it', async () => {
  const { frames } = await rolls(requesting(), { blocks: [...asked, step], activity: ANSWERED })
  expect(frames.flat().length).toBeGreaterThan(0)
})

test('an error takes the Requesting row\'s place without a roll', async () => {
  const failed = { type: 'error', id: 'e-1', createdAt, model, message: 'The provider request failed.', code: null } as Block
  expect(await rolls(requesting(), { blocks: [...asked, failed], phase: 'idle', activity: ANSWERED })).toEqual({ frames: [[], [], []], tail: null })
})

test('Continue rolls Requesting over the Stopped row', async () => {
  const stopped = { type: 'abort', id: 'x-1', createdAt, model, isResumed: false } as Block
  const before: ListState = { blocks: [...asked, stopped], phase: 'idle', activity: ANSWERED, pendingAction: null }
  // The session view tells the press both ways (`rootActivity`).
  const { frames, tail } = await rolls(before, { pendingAction: 'resume', activity: { ...ANSWERED, actionSent: true } })
  expect({ rolled: frames.flat().length > 0, tail }).toEqual({ rolled: true, tail: t('transcript.thinking.requesting') })
})
