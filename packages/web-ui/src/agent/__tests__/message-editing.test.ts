import { expect, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import { reactive, toRaw } from 'vue'
import type { Block } from '@demicodes/protocol'
import {
  beginMessageEdit,
  changeMessageEditContent,
  EditRejectedError,
  messageEditSuffixIds,
  lastEditableUserMessageId,
  restoreMessageEdit,
  sentEditRequest,
  submitMessageEdit,
  type MessageEditState,
  type MessageEditRequest,
} from '../message-editing'
import { createdAt, model, userBlock } from './agent-harness'

const SHA = (digit: string) => digit.repeat(64)

test('the UI selects only the last user block, whatever follows it', () => {
  const blocks: Block[] = [userBlock('A', 'A', 'A'), userBlock('B', 'B', 'B'), userBlock('C', 'C', 'C')]
  expect(lastEditableUserMessageId(blocks)).toBe('C')
  blocks.push(
    { type: 'wakeup', id: 'wakeup', turnId: 'wakeup', createdAt, model, placement: 'new_turn' },
    { type: 'steer', id: 'steer', turnId: 'wakeup', createdAt, model, content: [] },
    { type: 'text', id: 'reply', createdAt, model, text: 'answer C' },
  )
  expect(lastEditableUserMessageId(blocks)).toBe('C')
  expect(lastEditableUserMessageId(blocks.slice(3))).toBeNull()
  expect(lastEditableUserMessageId([])).toBeNull()
  expect(lastEditableUserMessageId([userBlock('A', 'A', 'A'), userBlock('replacement', 'r', 'r')])).toBe('replacement')
})

test('an edit keeps the message\'s files by reference and names added files by their upload', () => {
  const edit = beginMessageEdit({
    ...userBlock('user-B', 'turn-B', 'unused'),
    content: [
      { type: 'text', text: 'compare' },
      { type: 'image', source: { type: 'ref', ref: SHA('a'), mediaType: 'image/png' } },
      {
        type: 'attachment', name: 'chart.png', path: '/home/demi/.demi/attachments/c/chart.png',
        mediaType: 'image/png', sizeBytes: 4, sha256: SHA('a'),
      },
      { type: 'document', source: { type: 'ref', ref: SHA('b'), mediaType: 'application/pdf', fileName: 'a.pdf' } },
      { type: 'reference', reference: 'file:///notes.md?host=laptop' },
    ],
  }, { epoch: 'epoch', revision: 4 })
  edit.request.content.push({
    type: 'upload', ref: 'upload-1', fileName: 'new.txt', mediaType: 'text/plain', sha256: SHA('c'), snippet: 'new',
  })
  expect(sentEditRequest(edit.request).content).toEqual([
    { type: 'text', text: 'compare' },
    { type: 'media', media: { type: 'image', ref: SHA('a'), mediaType: 'image/png' } },
    { type: 'attachment', path: '/home/demi/.demi/attachments/c/chart.png' },
    { type: 'media', media: { type: 'document', ref: SHA('b'), mediaType: 'application/pdf', fileName: 'a.pdf' } },
    { type: 'reference', reference: 'file:///notes.md?host=laptop' },
    { type: 'upload', ref: 'upload-1', fileName: 'new.txt' },
  ])
})

test('media that is not a blob of the conversation refuses the edit before it is sent', () => {
  const edit = beginMessageEdit({
    ...userBlock('user-B', 'turn-B', 'unused'),
    content: [{ type: 'image', source: { type: 'binary', data: 'AAAA', mediaType: 'image/png' } }],
  }, { epoch: 'epoch', revision: 4 })
  expect(() => sentEditRequest(edit.request)).toThrow(EditRejectedError)
})

test('only the target and its suffix are muted; an accepted rewrite removes the old cut', () => {
  const blocks = ['A', 'answer-A', 'B', 'answer-B', 'C'].map((id) => ({ id }))
  expect([...messageEditSuffixIds(blocks, 'B')]).toEqual(['B', 'answer-B', 'C'])
  expect([...messageEditSuffixIds(blocks, 'A')]).toEqual(blocks.map((block) => block.id))
  expect([...messageEditSuffixIds(blocks, undefined)]).toEqual([])
  expect([...messageEditSuffixIds([{ id: 'A' }, { id: 'B-edited' }], 'B')]).toEqual([])
})

function draft(): MessageEditState {
  return beginMessageEdit({
    ...userBlock('user-B', 'turn-B', 'unused'),
    content: [
      { type: 'text', text: 'first\nsecond' },
      { type: 'document', source: { type: 'ref', ref: SHA('d'), fileName: 'a.pdf', mediaType: 'application/pdf' } },
      { type: 'text', text: 'third' },
    ],
  }, { epoch: 'epoch', revision: 4 })
}

test('editing a reactive multipart draft remains serializable and preserves the other attachments', () => {
  const original = reactive(draft())
  const changed = changeMessageEditContent(original, (content) => {
    const first = content[0]!
    if (first.type === 'text') first.text = 'updated'
  })
  const saved = structuredClone(toRaw(reactive(changed)))
  expect(saved.request.content.slice(1)).toEqual(toRaw(original.request.content).slice(1))
  expect(original.request.content[0]).toEqual({ type: 'text', text: 'first\nsecond' })
})

test('duplicate save is blocked and the complete detached draft is retained until acceptance', async () => {
  let state: MessageEditState | null = draft()
  const original = state
  const accepted = deferred<void>()
  const requests: MessageEditRequest[] = []
  const host = {
    get: () => state,
    set: (next: MessageEditState | null) => { state = next },
    send: async (request: MessageEditRequest) => {
      requests.push(request)
      await accepted.promise
    },
  }
  const first = submitMessageEdit(host)
  try {
    await submitMessageEdit(host)
    expect(requests).toHaveLength(1)
    expect(state?.phase).toBe('sending')
    expect(requests[0]!.content).toEqual(original.request.content)
    original.request.content.splice(0)
    expect(requests[0]!.content).toHaveLength(3)
  } finally {
    accepted.resolve()
    await first
  }
  expect(state).toBeNull()
})

test('lost confirmation survives reload and retries exactly the same operation and content', async () => {
  let state: MessageEditState | null = draft()
  const request = structuredClone(state.request)
  const requests: MessageEditRequest[] = []
  const host = {
    get: () => state,
    set: (next: MessageEditState | null) => { state = next },
    send: async (input: MessageEditRequest) => {
      requests.push(input)
      if (requests.length === 1) throw new Error('connection lost')
    },
  }
  await submitMessageEdit(host)
  expect(state?.phase).toBe('uncertain')
  state = restoreMessageEdit(structuredClone(state))
  await submitMessageEdit(host)
  expect(requests).toEqual([request, request])
  expect(state).toBeNull()
  expect(restoreMessageEdit({ ...draft(), phase: 'sending' })?.phase).toBe('uncertain')
})

test('explicit rejection restores editing with the draft and attachments intact', async () => {
  let state: MessageEditState | null = draft()
  const request = structuredClone(state.request)
  await submitMessageEdit({
    get: () => state,
    set: (next) => { state = next },
    send: async () => { throw new EditRejectedError('save failed') },
  })
  expect(state).toEqual({ phase: 'editing', request })
})

test('a late result cannot clear a different editor or conversation draft', async () => {
  let state: MessageEditState | null = draft()
  const accepted = deferred<void>()
  const pending = submitMessageEdit({
    get: () => state,
    set: (next) => { state = next },
    send: () => accepted.promise,
  })
  const other = draft()
  state = other
  accepted.resolve()
  await pending
  expect(state).toBe(other)
})
