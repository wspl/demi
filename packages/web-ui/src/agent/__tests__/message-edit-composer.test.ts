import { expect, test } from 'bun:test'
import { effectScope, ref } from 'vue'
import { deferred } from '@demicodes/utils'
import { useMessageEditComposer } from '../message-input/useMessageEditComposer'
import type { MessageEditState } from '../message-editing'

function fixture() {
  const state = ref<MessageEditState | null>({
    phase: 'editing',
    request: {
      operationId: 'edit', targetBlockId: 'B', version: { epoch: 'epoch', revision: 1 },
      content: [
        { type: 'text', text: 'first\nparagraph' },
        { type: 'document', source: { fileName: 'same.pdf', mediaType: 'application/pdf', data: new Uint8Array([1]) } },
        { type: 'text', text: 'second paragraph' },
        { type: 'document', source: { fileName: 'same.pdf', mediaType: 'application/pdf', data: new Uint8Array([2]) } },
      ],
    },
  })
  const scope = effectScope()
  const editor = scope.run(() => useMessageEditComposer({
    state: () => state.value,
    update: (value) => { state.value = value },
    root: ref<HTMLElement>(),
  }))!
  return { state, editor, close: () => scope.stop() }
}

test('composer edits retain multipart order and remove the chosen same-name attachment', () => {
  const f = fixture()
  try {
    f.editor.changeText(2, 'updated second paragraph')
    f.editor.removeAttachment(1)
    expect(f.state.value!.request.content).toEqual([
      { type: 'text', text: 'first\nparagraph' },
      { type: 'text', text: 'updated second paragraph' },
      { type: 'document', source: { fileName: 'same.pdf', mediaType: 'application/pdf', data: new Uint8Array([2]) } },
    ])
    f.editor.cancel()
    expect(f.state.value).toBeNull()
  } finally {
    f.close()
  }
})

test('sending and uncertain requests cannot be modified or discarded from the composer', () => {
  const f = fixture()
  try {
    for (const phase of ['sending', 'uncertain'] as const) {
      f.state.value!.phase = phase
      f.editor.changeText(0, 'must not change')
      f.editor.removeAttachment(1)
      f.editor.cancel()
      expect(f.state.value!.request.content).toHaveLength(4)
      expect(f.state.value!.request.content[0]).toEqual({ type: 'text', text: 'first\nparagraph' })
    }
  } finally {
    f.close()
  }
})

test('new attachment bytes append without replacing text changed while reading', async () => {
  const f = fixture()
  try {
    const pending = f.editor.addFiles([new File(['%PDF-new'], 'same.pdf', { type: 'application/pdf' })])
    f.editor.changeText(0, 'typed during read')
    await pending
    expect(f.state.value!.request.content[0]).toEqual({ type: 'text', text: 'typed during read' })
    expect(f.state.value!.request.content.at(-1)).toEqual({
      type: 'document', source: { fileName: 'same.pdf', mediaType: 'application/pdf', data: new TextEncoder().encode('%PDF-new') },
    })
    expect(f.editor.reading.value).toBeNull()
  } finally {
    f.close()
  }
})

test('cancel and unmount abort an attachment read without a late draft update', async () => {
  for (const end of ['cancel', 'unmount']) {
    const f = fixture()
    const bytes = deferred<ArrayBuffer>()
    const file = new File(['%PDF'], 'new.pdf')
    file.arrayBuffer = () => bytes.promise
    const pending = f.editor.addFiles([file])
    if (end === 'cancel') {
      f.editor.cancel()
    } else {
      f.close()
    }
    bytes.resolve(new ArrayBuffer(4))
    await pending
    expect(f.state.value?.request.content.length ?? 0).toBe(end === 'cancel' ? 0 : 4)
    expect(f.editor.reading.value).toBeNull()
    f.close()
  }
})
