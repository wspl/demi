import { expect, test } from 'bun:test'
import type { UserContentBlock } from '@demicodes/protocol'
import { ATTACHMENT_MARK as MARK, parseUserMarkdown } from '../../markdown/user-markdown'
import { contentCapsule } from '../message-editor/capsules'
import { encodeRemoteReference } from '../message-input/attachments'
import {
  messageClipboard,
  pasteMessage,
  readCopiedMessage,
  type CopiedMessage,
  type PasteHost,
} from '../message-input/copied-message'
import { splitMessageContent } from '../message-input/message-content'

// A sent message on the clipboard and back in a composer (`product.md`
// § Writing a message): Demi's own form carries each file as the message
// holds it, so a paste brings the files back as capsules without their
// bytes. About 10 ms in all.

const BEFORE = 'b'.repeat(64)
const AFTER = 'a'.repeat(64)
const FITTED = 'f'.repeat(64)

/** A message the transcript holds: two images, each its fitted picture and record, and a file on a paired device. */
const sent: UserContentBlock[] = [
  { type: 'text', text: 'Compare **before** ' },
  { type: 'image', source: { type: 'ref', ref: FITTED, mediaType: 'image/png' } },
  { type: 'attachment', name: 'before.png', path: '/home/demi/.demi/attachments/c/before.png', mediaType: 'image/png', sizeBytes: 1200, sha256: BEFORE },
  { type: 'text', text: ' with ' },
  { type: 'attachment', name: 'after.png', path: '/home/demi/.demi/attachments/c/after.png', mediaType: 'image/png', sizeBytes: 900, sha256: AFTER },
  { type: 'text', text: ', notes in ' },
  { type: 'reference', reference: encodeRemoteReference('alex-mbp', '/Users/alex/notes.md', 'device-1') },
]

/** The sent bubble's editor document, built as `UserBlock` builds it. */
function sentDocument() {
  const { markdown, files } = splitMessageContent(sent)
  return parseUserMarkdown(markdown, files.map((blocks, index) => contentCapsule(String(index), blocks)))
}

const copied: CopiedMessage = {
  text: `Compare **before** ${MARK} with ${MARK}, notes in ${MARK}`,
  files: [
    { type: 'stored', name: 'before.png', sha256: BEFORE, mediaType: 'image/png', size: 1200 },
    { type: 'stored', name: 'after.png', sha256: AFTER, mediaType: 'image/png', size: 900 },
    { type: 'device', name: 'notes.md', deviceId: 'device-1', host: 'alex-mbp', path: '/Users/alex/notes.md' },
  ],
}

/** A composer's taking of files that records what it was asked, claiming the blobs in `held`. */
function composerHost(held: Set<string>) {
  const added: File[] = []
  const claimed: string[] = []
  const host: PasteHost = {
    claim: async (file) => {
      claimed.push(file.sha256)
      return held.has(file.sha256) ? { id: `upload-${file.name}`, name: file.name } : null
    },
    remote: (file) => ({ id: `remote-${file.name}`, name: file.name, host: file.host, path: file.path }),
    add: (files) => {
      added.push(...files)
      return files.map((file) => ({ id: `pasted-${file.name}`, name: file.name }))
    },
  }
  return { host, added, claimed }
}

/** A document with each capsule as its file's name, to compare documents whose capsules have other ids. */
function named(doc: ReturnType<typeof parseUserMarkdown>): unknown {
  return JSON.parse(JSON.stringify(doc, (key, value: unknown) =>
    key === 'capsule' && typeof value === 'object' && value && 'name' in value ? value.name : value))
}

test('a sent message copies as its Markdown with each file by name, and Demi\'s own form holds each file as the message does', () => {
  const clipboard = messageClipboard(sentDocument())
  expect(clipboard.plain).toBe('Compare **before** before.png with after.png, notes in notes.md')
  // The original, not the picture fitted for the model, and the device by its id.
  expect(clipboard.message).toEqual(copied)
  expect(clipboard.image).toBeNull()
  expect(readCopiedMessage(JSON.stringify(clipboard.message))).toEqual(copied)
})

test('a copy of a lone image carries the picture itself; a file still on its way copies as its name', () => {
  const { markdown, files } = splitMessageContent([sent[1]!, sent[2]!])
  const lone = messageClipboard(parseUserMarkdown(markdown, files.map((blocks) => contentCapsule('0', blocks))))
  expect(lone.image?.name).toBe('before.png')
  expect(lone.message.files).toHaveLength(1)

  const uploading = messageClipboard(parseUserMarkdown(`Look ${MARK}`, [{ id: 'local', name: 'shot.png', image: 'blob:shot' }]))
  expect(uploading.message).toEqual({ text: 'Look shot.png', files: [] })
})

test('a pasted copy brings the same message back, each file a capsule where it stood, without uploading any', async () => {
  const { host, added, claimed } = composerHost(new Set([BEFORE, AFTER]))
  const pasted = await pasteMessage(copied, null, host)
  expect(pasted.missing).toEqual([])
  expect(added).toEqual([])
  expect(claimed).toEqual([BEFORE, AFTER])
  expect(named(parseUserMarkdown(pasted.markdown, pasted.capsules))).toEqual(named(sentDocument()))
})

test('a file the paste cannot claim comes from the picture the copy carried, or else stands as its name', async () => {
  const picture = new File(['png'], 'image.png', { type: 'image/png' })
  const lone: CopiedMessage = { text: MARK, files: [copied.files[0]!] }
  const withPicture = composerHost(new Set())
  const fromPicture = await pasteMessage(lone, picture, withPicture.host)
  // Under its own name, not the one a web browser gives every pasted picture.
  expect(withPicture.added.map((file) => [file.name, file.size])).toEqual([['before.png', picture.size]])
  expect(fromPicture).toEqual({ markdown: MARK, capsules: [{ id: 'pasted-before.png', name: 'before.png' }], missing: [] })

  // Another account's blob, and no picture: the name stands in the text, for the toast to name.
  const { host } = composerHost(new Set([AFTER]))
  const named = await pasteMessage(copied, null, host)
  expect(named.markdown).toBe(`Compare **before** before.png with ${MARK}, notes in ${MARK}`)
  expect(named.capsules.map((capsule) => capsule.name)).toEqual(['after.png', 'notes.md'])
  expect(named.missing).toEqual(['before.png'])
})

test('Demi\'s own form is read only when it is one: JSON whose text has a mark for each file', () => {
  expect(readCopiedMessage(null)).toBeNull()
  expect(readCopiedMessage('not json')).toBeNull()
  expect(readCopiedMessage(JSON.stringify({ text: `one ${MARK}`, files: [] }))).toBeNull()
  expect(readCopiedMessage(JSON.stringify({ text: 'plain', files: [] }))).toEqual({ text: 'plain', files: [] })
})
