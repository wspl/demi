import { expect, test } from 'bun:test'
import { acceptAttribute, dataTransferFiles, fileMatchesAcceptedExtensions, filePreviewUrl, fileToUserContent, partitionAcceptedFiles, transferHasFiles } from '../message-input/attachments'

test('empty accepted list allows any file', () => {
  expect(fileMatchesAcceptedExtensions(new File(['x'], 'note.txt'), [])).toBe(true)
})

test('accepted extensions match the file suffix and jpeg/jpg', () => {
  expect(fileMatchesAcceptedExtensions(new File(['x'], 'shot.png'), ['png'])).toBe(true)
  expect(fileMatchesAcceptedExtensions(new File(['x'], 'shot.jpeg'), ['jpg'])).toBe(true)
  expect(fileMatchesAcceptedExtensions(new File(['x'], 'note.txt'), ['png'])).toBe(false)
})

test('a paste or drop splits into accepted and rejected files', () => {
  const png = new File(['x'], 'shot.png')
  const pdf = new File(['x'], 'spec.pdf')
  expect(partitionAcceptedFiles([png, pdf], ['png'])).toEqual({ accepted: [png], rejected: [pdf] })
  expect(partitionAcceptedFiles([png, pdf], [])).toEqual({ accepted: [png, pdf], rejected: [] })
})

test('image files get an object-url preview', () => {
  const png = filePreviewUrl(new File(['x'], 'shot.png', { type: 'image/png' }))
  expect(png?.startsWith('blob:')).toBe(true)
  if (png) URL.revokeObjectURL(png)
  expect(filePreviewUrl(new File(['x'], 'note.txt', { type: 'text/plain' }))).toBeUndefined()
})

test('dataTransfer files are the drop / paste list', () => {
  const file = new File(['x'], 'note.txt')
  const transfer = {
    files: [file],
    types: ['Files'],
  } as unknown as DataTransfer
  expect(transferHasFiles(transfer)).toBe(true)
  expect(transferHasFiles({ types: ['text/plain'] } as unknown as DataTransfer)).toBe(false)
  expect(transferHasFiles(null)).toBe(false)
  expect(dataTransferFiles(transfer)).toEqual([file])
})

test('accept attribute lists dotted extensions', () => {
  expect(acceptAttribute([])).toBeUndefined()
  expect(acceptAttribute(['png', 'pdf'])).toBe('.png,.pdf')
})

test('png magic bytes become an image block', async () => {
  const bytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0])
  const block = await fileToUserContent(new File([bytes], 'shot.png', { type: 'image/png' }))
  expect(block).toEqual({
    type: 'image',
    source: { type: 'binary', data: bytes, mediaType: 'image/png' },
  })
})

test('unknown bytes become a document block', async () => {
  const bytes = new Uint8Array([1, 2, 3, 4])
  const block = await fileToUserContent(new File([bytes], 'note.txt', { type: 'text/plain' }))
  expect(block.type).toBe('document')
  if (block.type !== 'document') return
  expect(block.source.fileName).toBe('note.txt')
  expect(block.source.mediaType.startsWith('text/plain')).toBe(true)
  expect(block.source.data).toEqual(bytes)
})
