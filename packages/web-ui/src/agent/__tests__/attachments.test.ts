import { expect, test } from 'bun:test'
import { delay } from '@demicodes/utils'
import {
  applyAttachmentUpdate,
  arrangeCapsuleFiles,
  AttachmentUploadQueue,
  attachmentFileError,
  attachmentProgress,
  attachmentsReady,
  attachmentSendBlockReason,
  clampUnit,
  composerAttachment,
  composerAttachmentFromFile,
  composerRemoteAttachment,
  decodeRemoteReference,
  encodeRemoteReference,
  fileNameFromPath,
  remoteAttachmentError,
  dataTransferFiles,
  filePreviewUrl,
  type AttachmentUploadUpdate,
} from '../message-input/attachments'
import { transferHasFiles } from '../../composables/useFileDrop'

test('image files get an object-url preview', () => {
  const png = filePreviewUrl(new File(['x'], 'shot.png', { type: 'image/png' }))
  expect(png?.startsWith('blob:')).toBe(true)
  if (png)
    URL.revokeObjectURL(png)
  expect(filePreviewUrl(new File(['x'], 'note.txt', { type: 'text/plain' }))).toBeUndefined()
})

test('dataTransfer files are the drop / paste list', () => {
  const file = new File(['x'], 'note.txt')
  const transfer = {
    files: [file],
    types: ['Files'],
  } as unknown as DataTransfer
  expect(transferHasFiles(transfer)).toBe(true)
  expect(transferHasFiles({ types: ['text/plain'] } as unknown as DataTransfer)).toBe(
    false
  )
  expect(transferHasFiles(null)).toBe(false)
  expect(dataTransferFiles(transfer)).toEqual([file])
})

test('a composer file starts uploading and becomes sendable only when every file is ready', () => {
  const file = composerAttachmentFromFile(
    new File(['x'], 'shot.png', { type: 'image/png' }),
  )
  expect(file.phase).toBe('uploading')
  expect(file.progress).toBe(0)
  expect(attachmentsReady([file])).toBe(false)
  expect(attachmentSendBlockReason([file.phase])).toBe(
    'Wait for attachments to finish uploading'
  )
  expect(attachmentSendBlockReason(['ready', 'failed', 'uploading'])).toBe(
    'Retry or remove the attachments that did not upload'
  )
  expect(attachmentsReady([])).toBe(true)
  expect(attachmentsReady([composerAttachment({ name: 'a', phase: 'ready' })])).toBe(
    true
  )
  expect(attachmentSendBlockReason(['ready'])).toBeUndefined()
})

test('progress is a 0–1 unit only while uploading', () => {
  expect(clampUnit(-1)).toBe(0)
  expect(clampUnit(0.42)).toBe(0.42)
  expect(clampUnit(2)).toBe(1)
  expect(attachmentProgress({ phase: 'ready', progress: 0.9 })).toBe(0)
  expect(attachmentProgress({ phase: 'uploading', progress: 0.42 })).toBe(0.42)
  const item = composerAttachment({ name: 'a', phase: 'uploading', progress: 0 })
  applyAttachmentUpdate(item, { phase: 'uploading', progress: 0.5 })
  expect(item.progress).toBe(0.5)
  applyAttachmentUpdate(item, { phase: 'ready' })
  expect(item.phase).toBe('ready')
  expect(item.progress).toBeUndefined()
})

test('a remote file is ready at once, and its reference names its host and full path', () => {
  const remote = composerRemoteAttachment(
    {
      host: 'zan-mbp',
      path: '/Users/zan/Projects/demi/package.json'
    }
  )
  expect(fileNameFromPath(remote.path)).toBe('package.json')
  expect(remote.name).toBe('package.json')
  expect(remote.kind).toBe('reference')
  expect(attachmentsReady([remote])).toBe(true)
  expect(remoteAttachmentError(remote.path, remote.host, [remote])).toBeDefined()
  expect(remoteAttachmentError(remote.path, 'build-01', [remote])).toBeUndefined()
  const encoded = encodeRemoteReference(remote.host, remote.path)
  expect(encoded.includes(remote.path)).toBe(true)
  expect(decodeRemoteReference(encoded)).toEqual({
    host: 'zan-mbp',
    path: remote.path,
    name: 'package.json',
  })
})

test('empty, oversized, and duplicate files are refused', () => {
  expect(attachmentFileError(new File([], 'empty.txt'), [])).toBeDefined()
  expect(attachmentFileError(new File(['x'], 'note.txt'), ['note.txt'])).toBe(
    'note.txt is already attached.'
  )
  expect(attachmentFileError(new File(['x'], 'note.txt'), [])).toBeUndefined()
})

test('the upload queue reports ready, rejects a failure, and silences a cancelled job', async () => {
  const queue = new AttachmentUploadQueue()
  const updates: AttachmentUploadUpdate[] = []
  const record = (update: AttachmentUploadUpdate) => updates.push({ ...update })

  expect(await queue.start('a', async (_signal, report) => {
    report(0.25)
    report(0.8)
  }, record)).toBe(true)
  expect(updates).toEqual([
    { phase: 'uploading', progress: 0 },
    { phase: 'uploading', progress: 0.25 },
    { phase: 'uploading', progress: 0.8 },
    { phase: 'ready' },
  ])

  updates.length = 0
  await expect(queue.start('b', async () => {
    throw new Error('no')
  }, record)).rejects.toThrow('no')
  expect(updates).toEqual([{ phase: 'uploading', progress: 0 }])

  updates.length = 0
  const cancelled = queue.start('c', async (signal) => delay(30, signal), record)
  queue.cancel('c')
  expect(await cancelled).toBe(false)
  expect(updates).toEqual([{ phase: 'uploading', progress: 0 }])
})

test('the message keeps the files of its capsules, and the composer carries the rest for an undo', () => {
  const before = { id: 'before' }
  const after = { id: 'after' }

  // The capsule of `after` is deleted: its file leaves the message and stops travelling.
  const deleted = arrangeCapsuleFiles([before, after], ['before', 'after'], ['before'])
  expect(deleted).toEqual({ carried: [before, after], stopped: [after], resumed: [] })

  // Undo brings the capsule back, and its file travels again, in its place.
  const undone = arrangeCapsuleFiles(deleted.carried, ['before'], ['after', 'before'])
  expect(undone).toEqual({ carried: [after, before], stopped: [], resumed: [after] })

  // A file of a message being sent is not the draft's to arrange.
  const sending = arrangeCapsuleFiles([before, after], ['before', 'after'], ['after'], (item) => item.id === 'before')
  expect(sending).toEqual({ carried: [after, before], stopped: [], resumed: [] })
})
