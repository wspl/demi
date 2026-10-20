import { expect, test } from 'bun:test'
import { effectScope, ref } from 'vue'
import { deferred } from '@demicodes/utils'
import { ATTACHMENT_MARK as MARK } from '../../markdown/user-markdown'
import { useMessageEditComposer } from '../message-input/useMessageEditComposer'
import type { UploadedFile, UploadFile } from '../message-input/attachments'
import { beginSavedEdit, editContent, type MessageEditState } from '../message-editing'

const SHA = (digit: string) => digit.repeat(64)

function pdf(digit: string) {
  return {
    type: 'document' as const,
    source: { type: 'ref' as const, ref: SHA(digit), mediaType: 'application/pdf', fileName: 'same.pdf' },
  }
}

/** An upload the test answers: each call waits until the test settles it. */
function uploads() {
  const calls: { file: File; answer: ReturnType<typeof deferred<UploadedFile>>; signal: AbortSignal }[] = []
  const upload: UploadFile = (file, options) => {
    const answer = deferred<UploadedFile>()
    calls.push({ file, answer, signal: options.signal })
    options.progress(0.5)
    return answer.promise
  }
  return { upload, calls }
}

function fixture(upload: UploadFile = uploads().upload) {
  const state = ref<MessageEditState | null>({
    kind: 'message',
    phase: 'editing',
    request: {
      operationId: 'edit', targetBlockId: 'B', version: { epoch: 'epoch', revision: 1 },
      content: [
        { type: 'text', text: 'first\nparagraph' },
        pdf('1'),
        { type: 'text', text: 'second paragraph' },
        pdf('2'),
      ],
    },
  })
  const scope = effectScope()
  const editor = scope.run(() => useMessageEditComposer({
    state: () => state.value,
    update: (value) => { state.value = value },
    upload,
    claim: async () => null,
  }))!
  return { state, editor, close: () => scope.stop() }
}

test('an edit opens as its text with a capsule where each file is, and a change keeps the order shown', () => {
  const f = fixture()
  try {
    expect(f.editor.markdown.value).toBe(`first\nparagraph${MARK}second paragraph${MARK}`)
    expect(f.editor.capsules.value.map((capsule) => capsule.name)).toEqual(['same.pdf', 'same.pdf'])
    // The first capsule is deleted, and the text around it joined and changed.
    f.editor.change(`first\nparagraph, then the second${MARK}`, [f.editor.capsules.value[1]!])
    expect(editContent(f.state.value!)).toEqual([
      { type: 'text', text: 'first\nparagraph, then the second' },
      pdf('2'),
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
      expect(editContent(f.state.value!)).toHaveLength(4)
      expect(editContent(f.state.value!)[0]).toEqual({ type: 'text', text: 'first\nparagraph' })
    }
  } finally {
    f.close()
  }
})

test('a file added to an edit is uploaded and joins the content once the backend has it', async () => {
  const server = uploads()
  const f = fixture(server.upload)
  try {
    const shown = [...f.editor.capsules.value]
    const [added] = f.editor.addFiles([new File(['notes'], 'notes.txt', { type: 'text/plain' })])
    f.editor.change(`typed during upload${MARK}${MARK}${MARK}`, [...shown, added!])
    expect(added!.name).toBe('notes.txt')
    expect(f.editor.transfers.transfer(added!.id)).toEqual({ phase: 'uploading', progress: 0.5 })
    expect(f.editor.sendBlockReason.value).toBe('Wait for attachments to finish uploading')
    // Until the upload is done the file is not part of what the edit would
    // send.
    expect(editContent(f.state.value!).map((part) => part.type)).toEqual(['text', 'document', 'document'])
    server.calls[0]!.answer.resolve({ id: 'upload-1', mediaType: 'text/plain', sha256: SHA('3'), snippet: 'notes' })
    await Bun.sleep(0)
    expect(editContent(f.state.value!).at(-1)).toEqual({
      type: 'upload', ref: 'upload-1', fileName: 'notes.txt', mediaType: 'text/plain', sha256: SHA('3'), snippet: 'notes',
    })
    expect(editContent(f.state.value!)[0]).toEqual({ type: 'text', text: 'typed during upload' })
    expect(f.editor.sendBlockReason.value).toBeUndefined()
  } finally {
    f.close()
  }
})

test('a file of a name the edit holds already joins it as a file of its own', () => {
  const server = uploads()
  const f = fixture(server.upload)
  try {
    const added = f.editor.addFiles([new File(['%PDF-3'], 'same.pdf', { type: 'application/pdf' })])
    expect(added.map((capsule) => capsule.name)).toEqual(['same.pdf'])
    expect(f.editor.attachmentError.value).toBeNull()
    expect(server.calls.map((call) => call.file.name)).toEqual(['same.pdf'])
  } finally {
    f.close()
  }
})

test('a failed upload keeps its capsule and blocks the edit until Retry uploads it', async () => {
  const server = uploads()
  const f = fixture(server.upload)
  try {
    const [added] = f.editor.addFiles([new File(['%PDF'], 'new.pdf', { type: 'application/pdf' })])
    f.editor.change(`first${MARK}`, [added!])
    server.calls[0]!.answer.reject(new Error('Upload connection failed'))
    await Bun.sleep(0)
    expect(f.editor.transfers.transfer(added!.id)).toEqual({ phase: 'failed' })
    expect(f.editor.attachmentError.value).toBe('new.pdf: Upload connection failed')
    expect(f.editor.sendBlockReason.value).toBe('Retry or remove the attachments that did not upload')
    f.editor.transfers.retry(added!.id)
    expect(server.calls).toHaveLength(2)
    server.calls[1]!.answer.resolve({ id: 'upload-2', mediaType: 'application/pdf', sha256: SHA('4') })
    await Bun.sleep(0)
    expect(f.editor.sendBlockReason.value).toBeUndefined()
    expect(editContent(f.state.value!).at(-1)).toMatchObject({ type: 'upload', ref: 'upload-2' })
  } finally {
    f.close()
  }
})

test('leaving the edit stops its uploads, and a late answer adds nothing', async () => {
  for (const end of ['cancel', 'unmount'] as const) {
    const server = uploads()
    const f = fixture(server.upload)
    f.editor.addFiles([new File(['%PDF'], 'new.pdf', { type: 'application/pdf' })])
    if (end === 'cancel') {
      f.editor.cancel()
    } else {
      f.close()
    }
    await Bun.sleep(0)
    expect(server.calls[0]!.signal.aborted).toBe(true)
    server.calls[0]!.answer.resolve({ id: 'late', mediaType: 'application/pdf', sha256: SHA('5') })
    await Bun.sleep(0)
    expect((f.state.value ? editContent(f.state.value) : []).some((part) => part.type === 'upload')).toBe(false)
    f.close()
  }
})

// A saved message's edit keeps what is typed as it is, blank space included;
// only a send applies the send's rules. Before, every change trimmed it.
test('a saved message\'s edit keeps its text as typed, blank space at its ends included', () => {
  const state = ref<MessageEditState | null>(beginSavedEdit('a', [{ type: 'text', text: '    code example\n' }]))
  const scope = effectScope()
  const editor = scope.run(() => useMessageEditComposer({
    state: () => state.value,
    update: (value) => { state.value = value },
    upload: uploads().upload,
    claim: async () => null,
  }))!
  try {
    expect(editor.markdown.value).toBe('    code example\n')
    editor.change('    code example\n\n', [])
    expect(editContent(state.value!)).toEqual([{ type: 'text', text: '    code example\n\n' }])
  } finally {
    scope.stop()
  }
})
