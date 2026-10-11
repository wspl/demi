import { expect, test } from 'bun:test'
import { Editor, type JSONContent } from '@tiptap/core'
import type { Slice } from '@tiptap/pm/model'
import type { UserContentBlock } from '@demicodes/protocol'
import { ATTACHMENT_MARK as MARK, parseUserMarkdown, serializeUserMarkdown } from '../../markdown/user-markdown'
import { contentCapsule } from '../message-editor/capsules'
import { markdownSlice } from '../message-editor/clipboard-slices'
import { composerExtensions } from '../message-editor/extensions'
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

/**
 * A composer's taking of files: each stored file a capsule at once, whose
 * attaching answers only when the test says, with the picture it was offered
 * recorded; a device file in `refused` is one the composer cannot take.
 */
function composerHost(refused: Set<string> = new Set()) {
  const claims: { name: string; picture: File | null; answer: (kept: boolean) => void }[] = []
  const host: PasteHost = {
    claim: (file, picture) => {
      const kept = Promise.withResolvers<boolean>()
      claims.push({ name: file.name, picture, answer: kept.resolve })
      return { capsule: { id: `claimed-${file.name}`, name: file.name }, kept: kept.promise }
    },
    remote: (file) => refused.has(file.path) ? null : { id: `remote-${file.name}`, name: file.name, host: file.host, path: file.path },
  }
  return { host, claims }
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

test('a pasted copy lands at once as the same message, each file a capsule where it stood, before any file is attached', async () => {
  const { host, claims } = composerHost()
  const pasted = pasteMessage(copied, null, host)
  // Nothing has answered: the document already holds every capsule.
  expect(named(parseUserMarkdown(pasted.markdown, pasted.capsules))).toEqual(named(sentDocument()))
  expect(pasted.missing).toEqual([])
  expect(claims.map((claim) => [claim.name, claim.picture])).toEqual([['before.png', null], ['after.png', null]])
  expect(pasted.attaching.map(({ id, name }) => [id, name])).toEqual([['claimed-before.png', 'before.png'], ['claimed-after.png', 'after.png']])
  // Each file's attaching answers on its own, under its capsule.
  claims[1]!.answer(false)
  claims[0]!.answer(true)
  expect(await Promise.all(pasted.attaching.map(({ kept }) => kept))).toEqual([true, false])
})

test('a copy of a lone file offers its picture under the file\'s name for when the paste cannot use the file; a device file the composer cannot take stands as its name', () => {
  const picture = new File(['png'], 'image.png', { type: 'image/png' })
  const lone = composerHost()
  pasteMessage({ text: MARK, files: [copied.files[0]!] }, picture, lone.host)
  // Under its own name, not the one a web browser gives every pasted picture.
  expect(lone.claims.map(({ picture }) => [picture?.name, picture?.size])).toEqual([['before.png', picture.size]])

  // A picture belongs to a copy of one file only; a device file the composer cannot take is its name.
  const { host, claims } = composerHost(new Set(['/Users/alex/notes.md']))
  const pasted = pasteMessage(copied, picture, host)
  expect(claims.map((claim) => claim.picture)).toEqual([null, null])
  expect(pasted.markdown).toBe(`Compare **before** ${MARK} with ${MARK}, notes in notes.md`)
  expect(pasted.missing).toEqual(['notes.md'])
})

/** What an empty composer holds once `paste` lands in it, as its Markdown. */
function pastedIntoEmptyComposer(paste: (schema: Editor['schema']) => Slice): string {
  const editor = new Editor({
    element: null,
    extensions: composerExtensions({ placeholder: '', submit: () => {}, cancel: () => true, editLast: () => false }),
    content: { type: 'doc', content: [{ type: 'paragraph' }] },
  })
  try {
    editor.view.dispatch(editor.state.tr.replaceSelection(paste(editor.schema)))
    return serializeUserMarkdown(editor.getJSON()).markdown
  } finally {
    editor.destroy()
  }
}

/** A task brief an agent wrote with a heredoc: its text ends with a line break, and a code block stands in it. */
const BRIEF = 'Fix the login test.\n\n```sh\nbun test auth\n```\n\nThen report.\n'

test('a copy holds no blank line before its first line or after its last, in each of its forms', () => {
  const whole = messageClipboard(parseUserMarkdown(`\n${BRIEF}`, []))
  expect(whole.plain).toBe(BRIEF.trim())
  expect(whole.message).toEqual({ text: BRIEF.trim(), files: [] })
  // The HTML is made from the same lines.
  expect(serializeUserMarkdown(whole.doc).markdown).toBe(BRIEF.trim())

  // A selection that ends at the end of the paragraph before the code: the
  // blank line between them is not part of the copy.
  const [intro] = parseUserMarkdown(BRIEF, []).content as JSONContent[]
  const selection = messageClipboard({ type: 'doc', content: [intro!] })
  expect(selection.plain).toBe('Fix the login test.')
  expect(serializeUserMarkdown(selection.doc).markdown).toBe('Fix the login test.')
})

test('a paste into a composer drops the blank lines at its start and end, keeping the lines between and their indentation', () => {
  expect(pastedIntoEmptyComposer((schema) => markdownSlice(schema, '\n\nfoo\n  bar\n\n'))).toBe('foo\n  bar')
  expect(pastedIntoEmptyComposer((schema) => markdownSlice(schema, ' \n\t\n  indented first\n\nlast\n   \n'))).toBe('  indented first\n\nlast')
})

test('a message copied and pasted into an empty composer is the message, with no line under it', () => {
  const { host } = composerHost()
  const copy = messageClipboard(parseUserMarkdown(BRIEF, []))
  const pasted = pasteMessage(copy.message, null, host)
  expect(pastedIntoEmptyComposer((schema) => markdownSlice(schema, pasted.markdown, pasted.capsules))).toBe(BRIEF.trim())
  // Its plain text pasted, as another app gives it back, is the message too.
  expect(pastedIntoEmptyComposer((schema) => markdownSlice(schema, `${copy.plain}\n`))).toBe(BRIEF.trim())
})

test('Demi\'s own form is read only when it is one: JSON whose text has a mark for each file', () => {
  expect(readCopiedMessage(null)).toBeNull()
  expect(readCopiedMessage('not json')).toBeNull()
  expect(readCopiedMessage(JSON.stringify({ text: `one ${MARK}`, files: [] }))).toBeNull()
  expect(readCopiedMessage(JSON.stringify({ text: 'plain', files: [] }))).toEqual({ text: 'plain', files: [] })
})
