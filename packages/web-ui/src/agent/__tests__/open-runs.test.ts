import { expect, test } from 'bun:test'
import { effectScope, nextTick, reactive } from 'vue'
import type { Block } from '@demicodes/protocol'
import { useMessageRows, type MessageRowsSource } from '../message-rows'
import { useOpenRuns } from '../open-runs'
import { createdAt, model, userBlock } from './agent-harness'

// A block brought into view inside a folded run, as a notice's title, a
// search result or a receipt's sender asks (`runtime.md` § Command
// reports): the run opens and the block's own row is the one shown and
// marked, never the run's row. The rows are the list's own, built from
// blocks as the backend stores them; the scroll and the mark are checked in
// the product. Pure: a few milliseconds.

const call = (id: string, run: string): Block => ({
  type: 'tool_call',
  id,
  run,
  createdAt,
  model,
  toolUseId: `${id}-use`,
  toolName: 'shell',
  status: 'completed',
  input: JSON.stringify({ script: `echo ${id}`, description: `Run ${id}` }),
  output: [],
  view: null,
})
const text = (id: string, body: string): Block => ({ type: 'text', id, createdAt, model, text: body })

function transcript(): MessageRowsSource {
  const seven = ['c-1', 'c-2', 'c-3', 'c-4', 'c-5', 'c-6', 'c-7'].map((id) => call(id, 'c-1'))
  return reactive<MessageRowsSource>({
    conversationId: 'c-1',
    blocks: [
      userBlock('u-1', 'm-1', 'Run the suites.'),
      ...seven,
      text('a-1', 'All seven ran.'),
      userBlock('u-2', 'm-2', 'And the lint?'),
      call('l-1', 'l-1'),
      text('a-2', 'Clean.'),
    ],
    pendingCalls: [],
    pendingSteers: [],
    queue: [],
    phase: 'idle',
    pendingSubmissions: [],
    atStart: true,
    atEnd: true,
    latest: true,
  })
}

test('a block folded in a run opens the run and is shown by its own row; the run stays open, and another transcript starts folded', async () => {
  const state = transcript()
  const scope = effectScope()
  const { rows, runs } = scope.run(() => {
    const list = useMessageRows(() => state)
    return { rows: list.rows, runs: useOpenRuns(() => list.rows.value, () => state.conversationId) }
  })!
  const runRow = (step: string) => rows.value.find((row) => row.type === 'work_group' && row.steps.some((item) => item.id === step))!.id
  const sevenRun = runRow('c-5')
  expect(runs.isOpen(sevenRun)).toBe(false)

  // The notice's call, the fifth of seven: its own row, in the opened run.
  expect(runs.reveal('c-5')).toEqual({ row: sevenRun, step: 'c-5' })
  expect(runs.isOpen(sevenRun)).toBe(true)
  // The run's first call is a step of it too, never the run's row.
  expect(runs.reveal('c-1')).toEqual({ row: sevenRun, step: 'c-1' })
  // A lone call's run is the call's own row, which shows as it is.
  expect(runs.reveal('l-1')).toEqual({ row: runRow('l-1'), step: null })
  expect(runs.isOpen(runRow('l-1'))).toBe(false)
  expect(runs.reveal('a-1')).toEqual({ row: 'a-1', step: null })
  expect(runs.reveal('gone')).toBeNull()

  // The reader folds it again as any run.
  runs.setOpen(sevenRun, false)
  expect(runs.isOpen(sevenRun)).toBe(false)
  runs.reveal('c-5')
  state.conversationId = 'c-2'
  await nextTick()
  expect(runs.isOpen(sevenRun)).toBe(false)
  scope.stop()
})
