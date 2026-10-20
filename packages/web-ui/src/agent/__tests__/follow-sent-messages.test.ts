import { expect, test } from 'bun:test'
import { effectScope, nextTick, shallowRef } from 'vue'
import { submissionRows } from '../list-tail'
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
