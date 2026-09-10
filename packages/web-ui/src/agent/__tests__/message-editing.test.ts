import { expect, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import { reactive, toRaw } from 'vue'
import type { Block } from '@demicodes/core'
import {
  beginMessageEdit,
  changeMessageEditContent,
  EditRejectedError,
  isMessageEditSubmitKey,
  restoreMessageEdit,
  submitMessageEdit,
  type MessageEditState,
  type MessageEditRequest,
} from '../message-editing'

test('edit keyboard submission permits multiline input and never submits an IME composition', () => {
  const enter = { key: 'Enter', metaKey: false, ctrlKey: false, isComposing: false }
  expect(isMessageEditSubmitKey(enter)).toBe(false)
  expect(isMessageEditSubmitKey({ ...enter, metaKey: true })).toBe(true)
  expect(isMessageEditSubmitKey({ ...enter, ctrlKey: true })).toBe(true)
  expect(isMessageEditSubmitKey({ ...enter, metaKey: true, isComposing: true })).toBe(false)
  expect(isMessageEditSubmitKey({ ...enter, ctrlKey: true, isComposing: true })).toBe(false)
  expect(isMessageEditSubmitKey({ ...enter, key: 'a', metaKey: true })).toBe(false)
})

function draft(): MessageEditState {
  return beginMessageEdit({
    type: 'user', id: 'user-B', turnId: 'turn-B', hidden: false,
    content: [
      { type: 'text', text: 'first\nsecond' },
      { type: 'document', source: { data: new Uint8Array([1, 2]), fileName: 'a.pdf', mediaType: 'application/pdf' } },
      { type: 'text', text: 'third' },
    ],
  } as Block, { epoch: 'epoch', revision: 4 })
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
  expect(state).toEqual({ phase: 'editing', request, error: 'save failed' })
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
