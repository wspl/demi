import { expect, test } from 'bun:test'
import { modelAcceptsMediaType, sniffModelMediaType, type Model } from '../index'

function model(acceptedExtensions: Model['acceptedExtensions']): Model {
  return {
    id: 'm',
    name: 'M',
    contextWindow: 1,
    inputLimit: null,
    thinking: [],
    acceptedExtensions,
  }
}

function bytes(...parts: Array<string | number[]>): Uint8Array {
  const out: number[] = []
  for (const part of parts) {
    if (typeof part === 'string') for (const ch of part) out.push(ch.charCodeAt(0))
    else out.push(...part)
  }
  while (out.length < 16) out.push(0)
  return new Uint8Array(out)
}

test('sniffModelMediaType detects the closed media set by magic bytes', () => {
  expect(sniffModelMediaType(bytes([0x89], 'PNG', [0x0d, 0x0a, 0x1a, 0x0a]))?.mediaType).toBe('image/png')
  expect(sniffModelMediaType(bytes([0xff, 0xd8, 0xff, 0xe0]))?.mediaType).toBe('image/jpeg')
  expect(sniffModelMediaType(bytes('GIF89a'))?.mediaType).toBe('image/gif')
  expect(sniffModelMediaType(bytes('RIFF', [1, 2, 3, 4], 'WEBP'))?.mediaType).toBe('image/webp')
  expect(sniffModelMediaType(bytes([0x1a, 0x45, 0xdf, 0xa3]))?.mediaType).toBe('video/webm')
  expect(sniffModelMediaType(bytes([0, 0, 0, 0x20], 'ftypisom'))?.mediaType).toBe('video/mp4')
  expect(sniffModelMediaType(bytes([0, 0, 0, 0x20], 'ftypqt  '))?.mediaType).toBe('video/quicktime')
  expect(sniffModelMediaType(bytes([0, 0, 0, 0x20], 'ftypM4V '))?.mediaType).toBe('video/x-m4v')

  // Outside the closed set: no guessing.
  expect(sniffModelMediaType(bytes('%PDF-1.7'))).toBeNull()
  expect(sniffModelMediaType(bytes('plain text here'))).toBeNull()
  expect(sniffModelMediaType(new Uint8Array([0x89]))).toBeNull()
})

test('modelAcceptsMediaType gates on catalog extensions', () => {
  const images = model(['png', 'jpg', 'jpeg', 'gif', 'webp'])
  expect(modelAcceptsMediaType(images, 'image/png')).toBe(true)
  expect(modelAcceptsMediaType(images, 'video/mp4')).toBe(false)

  const video = model(['png', 'mp4', 'mov', 'webm', 'm4v'])
  expect(modelAcceptsMediaType(video, 'video/mp4')).toBe(true)
  expect(modelAcceptsMediaType(video, 'video/quicktime')).toBe(true)

  // jpg-only catalogs still accept image/jpeg.
  expect(modelAcceptsMediaType(model(['jpg']), 'image/jpeg')).toBe(true)
  expect(modelAcceptsMediaType(model([]), 'image/png')).toBe(false)
  expect(modelAcceptsMediaType(images, 'application/pdf')).toBe(false)
})
