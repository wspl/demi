import { expect, test } from 'bun:test'
import { effectScope, reactive } from 'vue'
import { useMessageRows, type MessageRowsSource } from '../../agent/message-rows'
import { createdAt, model, userBlock } from '../../agent/__tests__/agent-harness'
import { sharedRows, type SharedItem } from '../shared-items'

// A share's page as its list shows it (`sharing.md` § Steps not shared):
// each run of steps the share does not hold is one line in its place,
// after the row it follows, and a window that starts with one shows it
// first. Pure: a few milliseconds.

const text = (id: string, body: string): SharedItem => ({ type: 'text', id, createdAt, model, text: body })
const steps = (id: string, count: number): SharedItem => ({ type: 'steps_not_shared', id, count })

/** The rows the list shows of `items`, each as its type and, for a run of steps, its count. */
function shown(items: SharedItem[]): string[] {
  const { blocks, steps: lines } = sharedRows(items)
  const source = reactive<MessageRowsSource>({
    conversationId: 'share',
    blocks,
    stepsNotShared: lines,
    pendingSteers: [],
    queue: [],
    phase: 'idle',
    atStart: true,
    atEnd: true,
    latest: true,
  })
  const scope = effectScope()
  const rows = scope.run(() => useMessageRows(() => source))!.rows.value
  scope.stop()
  return rows.map((row) => (row.type === 'steps_not_shared' ? `steps ${row.count}` : row.type))
}

test('each run of steps not shared is one line where it stood, between what was said', () => {
  expect(shown([
    userBlock('u-1', 'm-1', 'Why did the deploy fail?'),
    steps('s-1', 3),
    text('a-1', 'Looking at the log.'),
    steps('s-2', 12),
    text('a-2', 'The check asks the old port.'),
    userBlock('u-2', 'm-2', 'Fix it.'),
    text('a-3', 'Fixed.'),
    steps('s-3', 2),
  ])).toEqual(['user', 'steps 3', 'text', 'steps 12', 'text', 'user', 'text', 'steps 2'])
  expect(shown([steps('s-0', 4), text('a-0', 'Done.')])).toEqual(['steps 4', 'text'])
})
