import { expect, test } from 'bun:test'
import { delay } from '@demicodes/utils'
import {
  applyAttachmentUpdate,
  AttachmentUploadQueue,
  attachmentCaption,
  attachmentFileError,
  attachmentProgress,
  attachmentsReady,
  attachmentSendBlockReason,
  clampUnit,
  composerAttachment,
  composerAttachmentFromFile,
  composerRemoteAttachment,
  contentBlockCaption,
  decodeRemoteReference,
  encodeRemoteReference,
  fileNameFromPath,
  remoteAttachmentError,
  dataTransferFiles,
  filePreviewUrl,
  fileToUserContent,
  transferHasFiles,
  type AttachmentUploadUpdate,
} from '../message-input/attachments'

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

test('png magic bytes become an image block', async () => {
  const bytes = new Uint8Array([
    0x89,
    0x50,
    0x4e,
    0x47,
    0x0d,
    0x0a,
    0x1a,
    0x0a,
    0,
    0,
    0,
    0
  ])
  const block = await fileToUserContent(new File([bytes], 'shot.png', { type: 'image/png' }))
  expect(block).toEqual({
    type: 'image',
    source: { type: 'binary', data: bytes, mediaType: 'image/png' },
  })
})

test('a composer file starts uploading and becomes sendable only when every file is ready', () => {
  const file = composerAttachmentFromFile(
    new File(['x'], 'shot.png', { type: 'image/png' }),
  )
  expect(file.phase).toBe('uploading')
  expect(file.progress).toBe(0)
  expect(attachmentsReady([file])).toBe(false)
  expect(attachmentSendBlockReason([file])).toBe(
    'Wait for attachments to finish uploading'
  )
  expect(attachmentsReady([])).toBe(true)
  expect(attachmentsReady([composerAttachment({ name: 'a', phase: 'ready' })])).toBe(
    true
  )
  expect(
    attachmentSendBlockReason([
      composerAttachment({ name: 'a', phase: 'ready' })
    ])
  ).toBeUndefined()
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

test('caption is the file name, or the upload progress, never a path', () => {
  expect(attachmentCaption(composerAttachment({ name: 'shot.png', phase: 'ready' }))).toBe('shot.png')
  expect(attachmentCaption(composerAttachment({ name: 'shot.png', phase: 'uploading' }))).toBe(
    'Uploading 0% · shot.png',
  )
  expect(
    attachmentCaption(composerAttachment({ name: 'spec.pdf', phase: 'uploading', progress: 0.42 })),
  ).toBe('Uploading 42% · spec.pdf')
})

test('a remote file is a ready tile whose tooltip identifies its host and full path', () => {
  const remote = composerRemoteAttachment(
    {
      host: 'zan-mbp',
      path: '/Users/zan/Projects/demi/package.json'
    }
  )
  expect(fileNameFromPath(remote.path)).toBe('package.json')
  expect(remote.name).toBe('package.json')
  expect(remote.kind).toBe('reference')
  expect(attachmentCaption(remote)).toBe('zan-mbp · /Users/zan/Projects/demi/package.json')
  expect(attachmentCaption(remote)).toContain(remote.path)
  expect(attachmentsReady([remote])).toBe(true)
  expect(attachmentSendBlockReason([remote])).toBeUndefined()
  expect(remoteAttachmentError(remote.path, remote.host, [remote])).toBeDefined()
  expect(remoteAttachmentError(remote.path, 'build-01', [remote])).toBeUndefined()
  const encoded = encodeRemoteReference(remote.host, remote.path)
  expect(encoded.includes(remote.path)).toBe(true)
  expect(decodeRemoteReference(encoded)).toEqual({
    host: 'zan-mbp',
    path: remote.path,
    name: 'package.json',
  })
  expect(contentBlockCaption({ type: 'reference', reference: encoded })).toBe(
    'zan-mbp · /Users/zan/Projects/demi/package.json'
  )
  expect(contentBlockCaption({
    type: 'document',
    source: {
      data: new Uint8Array(),
      mediaType: 'application/pdf',
      fileName: 'login-failure.pdf'
    },
  })).toBe('login-failure.pdf')
})

test('empty, oversized, and duplicate files are refused', () => {
  expect(attachmentFileError(new File([], 'empty.txt'), [])).toBeDefined()
  expect(attachmentFileError(new File(['x'], 'note.txt'), ['note.txt'])).toBe(
    'note.txt is already attached.'
  )
  expect(attachmentFileError(new File(['x'], 'note.txt'), [])).toBeUndefined()
})

test('the upload queue applies ready, drops a failure, and ignores a cancelled job', async () => {
  const queue = new AttachmentUploadQueue()
  const updates: AttachmentUploadUpdate[] = []
  const dropped: string[] = []
  queue.start('a', async (_signal, report) => {
    report(0.25)
    report(0.8)
  }, (update) => updates.push({ ...update }), (id) => dropped.push(id))
  await delay(5)
  expect(updates).toEqual([
    { phase: 'uploading', progress: 0 },
    { phase: 'uploading', progress: 0.25 },
    { phase: 'uploading', progress: 0.8 },
    { phase: 'ready' },
  ])
  expect(dropped).toEqual([])

  updates.length = 0
  queue.start('b', async () => {
    throw new Error('no')
  }, (update) => updates.push({ ...update }), (id) => dropped.push(id))
  await delay(5)
  expect(updates).toEqual([{ phase: 'uploading', progress: 0 }])
  expect(dropped).toEqual(['b'])

  updates.length = 0
  dropped.length = 0
  queue.start(
    'c',
    async (signal) => delay(30, signal),
    (update) => updates.push({ ...update }),
    (id) => dropped.push(id)
  )
  queue.cancel('c')
  await delay(40)
  expect(updates).toEqual([{ phase: 'uploading', progress: 0 }])
  expect(dropped).toEqual([])
})

test('unknown bytes become a document block', async () => {
  const bytes = new Uint8Array([1, 2, 3, 4])
  const block = await fileToUserContent(new File([bytes], 'note.txt', { type: 'text/plain' }))
  expect(block.type).toBe('document')
  if (block.type !== 'document')
    return
  expect(block.source.fileName).toBe('note.txt')
  expect(block.source.mediaType.startsWith('text/plain')).toBe(true)
  expect(block.source.data).toEqual(bytes)
})
