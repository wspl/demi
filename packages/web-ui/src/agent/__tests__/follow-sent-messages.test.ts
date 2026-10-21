import { expect, test } from 'bun:test'
import { effectScope, nextTick, shallowRef } from 'vue'
import { listTailBlocks, submissionRows } from '../list-tail'
import type { MessageListBlock } from '../pending-steers'
import { rowKey } from '../row-key'
import { useFollowSentMessages } from '../useFollowSentMessages'
import { userBlock } from './agent-harness'

// A message the reader sends scrolls the list to the bottom. A message not
// delivered that shows where it was sent, as the page of history that holds its
// place is read, was not just sent, and the reader reading up stays where they
// read. Pure: a few ms.
test('a message not delivered that comes into view leaves the reader where they read, and its Retry follows to the bottom', async () => {
  const rows = shallowRef<MessageListBlock[]>([userBlock('u-1', 'm-1', 'Hello')])
  let scrolls = 0
  const scope = effectScope()
  scope.run(() => useFollowSentMessages(() => rows.value, rowKey, () => { scrolls += 1 }))
  const failed = { id: 'm-2', way: 'send' as const, text: 'With a file', attachments: [], error: 'The device is offline.' }
  rows.value = [...submissionRows(failed), ...rows.value]
  await nextTick()
  await nextTick()
  expect(scrolls).toBe(0)
  rows.value = [rows.value.at(-1)!, ...submissionRows({ ...failed, error: null })]
  await nextTick()
  await nextTick()
  expect(scrolls).toBe(1)
  scope.stop()
})

// The server may tell the message a turn starts with between the send and the
// message written. Before, that was a row of another name, and the list
// followed to the bottom twice more. Pure: a few ms.
test('a sent message the turn starts with follows to the bottom once, from its send', async () => {
  const before = [userBlock('u-1', 'm-1', 'Hello')]
  const rows = shallowRef<MessageListBlock[]>(before)
  let scrolls = 0
  const scope = effectScope()
  scope.run(() => useFollowSentMessages(() => rows.value, rowKey, () => { scrolls += 1 }))
  const submission = { id: 'm-2', way: 'send' as const, text: 'Go on.', attachments: [], error: null }
  const message = { id: 'm-2', content: [{ type: 'text' as const, text: 'Go on.' }] }
  const tail = (starting: typeof message | null, pendingSubmissions: (typeof submission)[]) =>
    listTailBlocks({ phase: 'running', blocks: before, pendingCalls: [], pendingSteers: [], queue: [], starting, pendingSubmissions }).turn
  for (const next of [
    tail(null, [submission]),
    tail(message, [submission]),
    tail(message, []),
    [userBlock('u-2', 'm-2', 'Go on.')],
  ]) {
    rows.value = [...before, ...next]
    await nextTick()
    await nextTick()
  }
  expect(scrolls).toBe(1)
  scope.stop()
})
