import { expect, test } from 'bun:test'
import { previewMediaType, showsInPlace } from '../index'

test('a file is known by its extension, whatever its case or separator', () => {
  expect(previewMediaType('/work/assets/logo.SVG')).toBe('image/svg+xml')
  expect(previewMediaType('C:\\work\\clip.m4v')).toBe('video/mp4')
  expect(previewMediaType('notes/README.md')).toBe('text/markdown')
  expect(previewMediaType('voice.opus')).toBe('audio/ogg')
  expect(previewMediaType('/work/app.bin')).toBeNull()
  expect(previewMediaType('/work/Makefile')).toBeNull()
  // A leading dot names a hidden file, not an extension.
  expect(previewMediaType('/work/.png')).toBeNull()
  expect(previewMediaType('/work.d/file')).toBeNull()
})

test('the page shows media in place and renders Markdown from its text', () => {
  expect(showsInPlace('image/svg+xml')).toBe(true)
  expect(showsInPlace('application/pdf')).toBe(true)
  expect(showsInPlace('audio/webm')).toBe(true)
  expect(showsInPlace('text/markdown')).toBe(false)
  expect(showsInPlace('text/html')).toBe(false)
})
