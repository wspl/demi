import { expect, test } from 'bun:test'
import type { UserContentBlock } from '@demicodes/core'
import { ATTACHMENT_MARK as MARK } from '../../markdown/user-markdown'
import { composerCapsule, contentCapsule } from '../message-editor/capsules'
import {
  composerAttachment,
  composerRemoteAttachment,
  encodeRemoteReference,
} from '../message-input/attachments'
import { joinMessageContent, splitMessageContent } from '../message-input/message-content'

// A message's content blocks and the editor's Markdown, with a mark where
// each file stands (`product.md` § Attachments).

const image: UserContentBlock = { type: 'image', source: { type: 'url', url: '/blobs/before.png' } }
const record: UserContentBlock = {
  type: 'attachment',
  name: 'before.png',
  path: '/home/demi/.demi/attachments/c/before.png',
  mediaType: 'image/png',
  sizeBytes: 10,
  sha256: 'a',
}
const log: UserContentBlock = {
  type: 'attachment',
  name: 'build.log',
  path: '/home/demi/.demi/attachments/c/build.log',
  mediaType: 'text/plain',
  sizeBytes: 20,
  sha256: 'b',
  snippet: 'error: the test failed',
}

test('a sent message reads as its text with a mark where each file was, a record together with its media', () => {
  const content: UserContentBlock[] = [
    { type: 'text', text: 'Compare ' },
    image,
    record,
    { type: 'text', text: ' with the log ' },
    log,
    { type: 'text', text: '.' },
  ]
  expect(splitMessageContent(content)).toEqual({
    markdown: `Compare ${MARK} with the log ${MARK}.`,
    files: [[image, record], [log]],
  })
  expect(joinMessageContent<UserContentBlock>(`Compare ${MARK} with the log ${MARK}.`, [[image, record], [log]])).toEqual(content)
})

test('text blocks that follow one another are paragraphs of one text', () => {
  expect(splitMessageContent([{ type: 'text', text: 'one' }, { type: 'text', text: 'two' }]).markdown).toBe('one\n\ntwo')
})

test('sent content is trimmed at its ends, a text of spaces between files is dropped, and a file without a mark follows', () => {
  expect(joinMessageContent<UserContentBlock>(`  Look ${MARK} ${MARK}\n`, [[record], [log], [image]])).toEqual([
    { type: 'text', text: 'Look ' },
    record,
    log,
    image,
  ])
  expect(joinMessageContent<UserContentBlock>(`${MARK}`, [])).toEqual([])
})

test('a capsule names its file, shows its picture or opening lines, and its upload', () => {
  expect(contentCapsule('0', [image, record])).toEqual({
    id: '0',
    name: 'before.png',
    image: image.type === 'image' ? image.source : undefined,
    path: record.type === 'attachment' ? record.path : undefined,
    snippet: undefined,
  })
  expect(contentCapsule('1', [log])).toMatchObject({ name: 'build.log', snippet: 'error: the test failed' })
  expect(contentCapsule('2', [{
    type: 'reference',
    reference: encodeRemoteReference('zan-mbp', '/Users/zan/Projects/demi/package.json'),
  }])).toMatchObject({ name: 'package.json', host: 'zan-mbp', path: '/Users/zan/Projects/demi/package.json' })
  expect(composerCapsule(composerAttachment({ id: 'u', name: 'shot.png', phase: 'uploading', progress: 0.42 })))
    .toMatchObject({ id: 'u', name: 'shot.png', upload: { phase: 'uploading', progress: 0.42 } })
  expect(composerCapsule(composerAttachment({ id: 'f', name: 'shot.png', phase: 'failed' })).upload)
    .toEqual({ phase: 'failed' })
  expect(composerCapsule(composerRemoteAttachment({ id: 'r', host: 'zan-mbp', path: '/a/b.txt' })))
    .toEqual({ id: 'r', name: 'b.txt', host: 'zan-mbp', path: '/a/b.txt' })
})
