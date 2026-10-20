import { expect, test } from 'bun:test'
import type { UserContentBlock } from '@demicodes/protocol'
import { ATTACHMENT_MARK as MARK } from '../../markdown/user-markdown'
import { composerCapsule, composerTransfer, contentCapsule } from '../message-editor/capsules'
import {
  composerAttachment,
  composerRemoteAttachment,
  encodeRemoteReference,
} from '../message-input/attachments'
import { joinMessageContent, sentMarkdown, splitMessageContent } from '../message-input/message-content'

// A message's content blocks and the editor's Markdown, with a mark where
// each file stands.

const image: UserContentBlock = { type: 'image', source: { type: 'url', url: '/blobs/before.png' } }
const record: UserContentBlock = {
  type: 'attachment',
  name: 'before.png',
  id: 'a1',
  mediaType: 'image/png',
  sizeBytes: 10,
  sha256: 'a',
}
const log: UserContentBlock = {
  type: 'attachment',
  name: 'build.log',
  id: 'a2',
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
  if (image.type !== 'image' || record.type !== 'attachment') {
    throw new Error('The fixtures are an image and its attachment record')
  }
  // A sent image shows as its thumbnail, from the picture the model read, and
  // holds the original file, which a copy of the message carries.
  expect(contentCapsule('0', [image, record])).toEqual({
    id: '0',
    name: 'before.png',
    image: image.source,
    snippet: undefined,
    reference: { type: 'stored', sha256: record.sha256, mediaType: record.mediaType, size: record.sizeBytes },
  })
  // An image the model could not read natively shows its own bytes.
  expect(contentCapsule('3', [{
    type: 'attachment', name: 'shot.png', id: 'a3',
    mediaType: 'image/png', sizeBytes: 3, sha256: 'b'.repeat(64),
  }]).image).toEqual({ type: 'ref', ref: 'b'.repeat(64), mediaType: 'image/png' })
  expect(contentCapsule('1', [log])).toMatchObject({ name: 'build.log', snippet: 'error: the test failed' })
  expect(contentCapsule('2', [{
    type: 'reference',
    reference: encodeRemoteReference('alex-mbp', '/Users/alex/Projects/demi/package.json'),
  }])).toMatchObject({ name: 'package.json', host: 'alex-mbp', path: '/Users/alex/Projects/demi/package.json' })
  expect(composerCapsule(composerAttachment({ id: 'u', name: 'shot.png', phase: 'uploading', progress: 0.42 })))
    .toEqual({ id: 'u', name: 'shot.png' })
  // How far a file has come is not a fact of the message: it travels beside the
  // capsule.
  expect(composerTransfer(composerAttachment({ id: 'u', name: 'shot.png', phase: 'uploading', progress: 0.42 })))
    .toEqual({ phase: 'uploading', progress: 0.42 })
  expect(composerTransfer(composerAttachment({ id: 'f', name: 'shot.png', phase: 'failed' })))
    .toEqual({ phase: 'failed' })
  expect(composerTransfer(composerAttachment({ id: 'r', name: 'shot.png', phase: 'ready' })))
    .toBeUndefined()
  expect(composerCapsule(composerRemoteAttachment({ id: 'r', host: 'alex-mbp', path: '/a/b.txt' })))
    .toEqual({ id: 'r', name: 'b.txt', host: 'alex-mbp', path: '/a/b.txt' })
})

// A message being sent is drawn as the message it becomes: a bubble one line
// taller until the server confirms it, for a newline an input method's Enter
// left in the draft, shrinks the list.
test('a message being sent shows the text its send carries, without the line breaks and spaces the send drops', () => {
  expect(sentMarkdown('2b: 按现在的规则。\n', 0)).toBe('2b: 按现在的规则。')
  expect(sentMarkdown(`\n  Compare ${MARK} \n${MARK}\n\n`, 2)).toBe(`Compare ${MARK}${MARK}`)
  // What it shows is what the confirmed message's blocks read as.
  const draft = `Look at ${MARK} please\n`
  expect(sentMarkdown(draft, 1)).toBe(splitMessageContent(joinMessageContent(draft, [[log]])).markdown)
})
