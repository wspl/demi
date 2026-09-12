import { expect, test } from 'bun:test'
import { attachmentTag, attachmentSnippet, isTextAttachment } from '../index'

test('an attachment block renders as one self-closing tag with escaped attributes', () => {
  expect(attachmentTag({
    type: 'attachment',
    name: 'notes "v2".md',
    path: '/home/demi/.demi/attachments/c1/notes "v2".md',
    mediaType: 'text/markdown',
    sizeBytes: 82,
    sha256: 'abc',
    snippet: 'never rendered',
  })).toBe(
    '<attachment name="notes &quot;v2&quot;.md" type="text/markdown" size="82" path="/home/demi/.demi/attachments/c1/notes &quot;v2&quot;.md"/>',
  )
})

test('text attachments are told by media type or extension and their snippet is the opening', () => {
  expect(isTextAttachment('a.log', 'application/octet-stream')).toBe(true)
  expect(isTextAttachment('a.bin', 'text/plain; charset=utf-8')).toBe(true)
  expect(isTextAttachment('a.bin', 'application/octet-stream')).toBe(false)
  expect(attachmentSnippet('  \r\nfirst\r\nsecond')).toBe('first\nsecond')
})
