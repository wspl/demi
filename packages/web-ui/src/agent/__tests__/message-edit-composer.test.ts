import { expect, test } from 'bun:test'
import { effectScope, ref } from 'vue'
import { deferred } from '@demicodes/utils'
import { ATTACHMENT_MARK as MARK } from '../../markdown/user-markdown'
import { useMessageEditComposer } from '../message-input/useMessageEditComposer'
import type { MessageEditState } from '../message-editing'

function pdf(byte: number) {
  return { type: 'document' as const, source: { fileName: 'same.pdf', mediaType: 'application/pdf', data: new Uint8Array([byte]) } }
}

function fixture() {
  const state = ref<MessageEditState | null>({
    phase: 'editing',
    request: {
      operationId: 'edit', targetBlockId: 'B', version: { epoch: 'epoch', revision: 1 },
      content: [
        { type: 'text', text: 'first\nparagraph' },
        pdf(1),
        { type: 'text', text: 'second paragraph' },
        pdf(2),
      ],
    },
  })
  const scope = effectScope()
  const editor = scope.run(() => useMessageEditComposer({
    state: () => state.value,
    update: (value) => { state.value = value },
  }))!
  return { state, editor, close: () => scope.stop() }
}

test('an edit opens as its text with a capsule where each file is, and a change keeps the order shown', () => {
  const f = fixture()
  try {
    expect(f.editor.markdown.value).toBe(`first\nparagraph${MARK}second paragraph${MARK}`)
    expect(f.editor.capsules.value.map((capsule) => capsule.name)).toEqual(['same.pdf', 'same.pdf'])
    const second = f.editor.capsules.value[1]!.id
    // The first capsule is deleted, and the text around it joined and changed.
    f.editor.change(`first\nparagraph, then the second${MARK}`, [second])
    expect(f.state.value!.request.content).toEqual([
      { type: 'text', text: 'first\nparagraph, then the second' },
      pdf(2),
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
      f.editor.change('must not change', [])
      f.editor.cancel()
      expect(f.state.value!.request.content).toHaveLength(4)
      expect(f.state.value!.request.content[0]).toEqual({ type: 'text', text: 'first\nparagraph' })
    }
  } finally {
    f.close()
  }
})

test('a file read into an edit becomes a capsule for the editor to place, and text typed meanwhile stays', async () => {
  const f = fixture()
  try {
    const ids = f.editor.capsules.value.map((capsule) => capsule.id)
    const pending = f.editor.addFiles([new File(['%PDF-new'], 'new.pdf', { type: 'application/pdf' })])
    f.editor.change(`typed during read${MARK}${MARK}`, ids)
    await pending
    const added = f.editor.capsules.value.at(-1)!
    expect(added.name).toBe('new.pdf')
    expect(f.state.value!.request.content[0]).toEqual({ type: 'text', text: 'typed during read' })
    f.editor.change(`typed during read${MARK}${MARK}${MARK}`, [...ids, added.id])
    expect(f.state.value!.request.content.at(-1)).toEqual({
      type: 'document', source: { fileName: 'new.pdf', mediaType: 'application/pdf', data: new TextEncoder().encode('%PDF-new') },
    })
    expect(f.editor.reading.value).toBeNull()
  } finally {
    f.close()
  }
})

test('cancel and unmount abort an attachment read without a late capsule', async () => {
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
    expect(f.editor.capsules.value).toHaveLength(end === 'cancel' ? 0 : 2)
    expect(f.editor.reading.value).toBeNull()
    f.close()
  }
})
