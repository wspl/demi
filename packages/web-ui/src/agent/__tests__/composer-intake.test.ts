import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'
import { Editor, type JSONContent } from '@tiptap/core'
import { NodeSelection } from '@tiptap/pm/state'
import { ATTACHMENT_MARK as MARK, parseUserMarkdown, serializeUserMarkdown } from '../../markdown/user-markdown'
import { composerExtensions } from '../message-editor/extensions'
import type { IntakeOptions } from '../message-editor/intake'
import { attachmentDrag, type CopiedMessage } from '../message-input/copied-message'

// What a composer takes in (`product.md` § Attachments): whatever is pasted
// or dropped becomes text in the user dialect or a capsule, never a picture
// set into the text or a node of another kind. The events go through
// ProseMirror's own handlers on a page of happy-dom, which lays nothing out,
// so a drop says where it lands instead of where the pointer is. About
// 250 ms in all, most of it the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })

/** What the composer handed its owner. */
interface Taken {
  files: File[][]
  messages: { message: CopiedMessage; picture: File | null }[]
  content: Promise<{ markdown: string; files: File[] }>
}

let open: Editor | null = null
afterEach(() => {
  open?.destroy()
  open = null
})

const PNG = new File([new Uint8Array([0x89, 0x50, 0x4e, 0x47])], 'shot.png', { type: 'image/png' })

/**
 * A composer on the page holding `markdown`, whose capsules' files it
 * carries; `readable` are the image addresses whose bytes the page can read.
 */
function composer(markdown = '', capsules: { id: string; name: string }[] = [], readable: Record<string, File> = {}) {
  let content!: (value: { markdown: string; files: File[] }) => void
  const taken: Taken = { files: [], messages: [], content: new Promise((resolve) => (content = resolve)) }
  const intake: IntakeOptions = {
    carries: (id) => capsules.some((capsule) => capsule.id === id),
    land: () => {},
    files: (files) => taken.files.push(files),
    message: (message, picture) => taken.messages.push({ message, picture }),
    content: (markdown, files) => content({ markdown, files }),
    readImage: async (src) => readable[src] ?? null,
  }
  const element = document.createElement('div')
  document.body.append(element)
  const editor = new Editor({
    element,
    extensions: composerExtensions({ placeholder: () => '', submit: () => {}, cancel: () => false, editLast: () => false, intake }),
    content: parseUserMarkdown(markdown, capsules),
    enableInputRules: false,
    enablePasteRules: false,
  })
  open = editor
  return { editor, taken }
}

function transfer(data: Record<string, string>, files: File[] = []): DataTransfer {
  const dataTransfer = new DataTransfer()
  for (const [type, value] of Object.entries(data)) {
    dataTransfer.setData(type, value)
  }
  for (const file of files) {
    dataTransfer.items.add(file)
  }
  return dataTransfer
}

function paste(editor: Editor, clipboardData: DataTransfer): void {
  editor.view.dom.dispatchEvent(new ClipboardEvent('paste', { clipboardData, bubbles: true, cancelable: true }))
}

/** Drops `dataTransfer` at `pos` of the document: the page has no layout to find it under the pointer. */
function drop(editor: Editor, dataTransfer: DataTransfer, pos: number): void {
  editor.view.posAtCoords = () => ({ pos, inside: -1 })
  editor.view.dom.dispatchEvent(dragEvent('drop', dataTransfer))
}

/** A drag event carrying `dataTransfer`, which happy-dom's constructor leaves out. */
function dragEvent(type: string, dataTransfer: DataTransfer): DragEvent {
  const event = new DragEvent(type, { bubbles: true, cancelable: true })
  Object.defineProperty(event, 'dataTransfer', { value: dataTransfer })
  return event
}

/** The node and mark types a document holds. */
function kinds(doc: JSONContent): Set<string> {
  const found = new Set<string>()
  const walk = (node: JSONContent) => {
    found.add(node.type ?? '')
    node.marks?.forEach((mark) => found.add(mark.type))
    node.content?.forEach(walk)
  }
  walk(doc)
  return found
}

function markdownOf(editor: Editor): string {
  return serializeUserMarkdown(editor.getJSON()).markdown
}

test('a pasted file or screenshot is a file the owner takes, never a picture in the text', () => {
  const { editor, taken } = composer('Look ')
  paste(editor, transfer({ 'text/html': '<img src="blob:https://demi.test/1">' }, [PNG]))
  expect(taken.files).toEqual([[PNG]])
  expect(kinds(editor.getJSON()).has('image')).toBe(false)
})

test('an image in dropped or pasted HTML whose bytes the page reads becomes a file where it stood', async () => {
  const data = 'data:image/png;base64,iVBORw0KGgo='
  const { editor, taken } = composer('Compare ', [], { [data]: PNG })
  drop(editor, transfer({ 'text/html': `<p>this <img src="${data}"> with that</p>`, 'text/plain': 'this  with that' }), 9)
  expect(await taken.content).toEqual({ markdown: `this ${MARK} with that`, files: [PNG] })
  expect(markdownOf(editor)).toBe('Compare ')
}, { timeout: 1000 })

test('an image another site does not let the page read becomes its link as text', async () => {
  const { editor, taken } = composer()
  paste(editor, transfer({ 'text/html': '<img src="https://other.example/cat.png" alt="A cat">' }))
  expect(await taken.content).toEqual({ markdown: 'https://other.example/cat.png', files: [] })
  expect(kinds(editor.getJSON()).has('image')).toBe(false)
}, { timeout: 1000 })

test('an image dragged from another page with its address as text is its link, not a picture', async () => {
  const { editor, taken } = composer()
  drop(editor, transfer({ 'text/html': '<img src="https://other.example/cat.png">', 'text/uri-list': 'https://other.example/cat.png' }), 1)
  expect(await taken.content).toEqual({ markdown: 'https://other.example/cat.png', files: [] })
}, { timeout: 1000 })

test('pasted rich text keeps the dialect\'s formats and of every other element only its text', () => {
  const { editor } = composer()
  paste(editor, transfer({
    'text/html': '<h1>Plan</h1><table><tr><td><b>Name</b></td><td><span style="color: red">Size</span></td></tr></table>'
      + '<ul><li>one <a href="https://demi.test/a">link</a></li></ul><p><i>it</i> and <code>x</code></p>',
    'text/plain': 'Plan\nName Size\none link\nit and x',
  }))
  expect(markdownOf(editor)).toBe('Plan\n**Name** Size\none [link](https://demi.test/a)\n*it* and `x`')
  expect([...kinds(editor.getJSON())].sort()).toEqual(['bold', 'code', 'doc', 'hardBreak', 'italic', 'link', 'paragraph', 'text'])
})

test('code copied from a code editor keeps its indentation, its colored HTML aside', () => {
  const { editor } = composer()
  paste(editor, transfer({
    'text/html': '<div style="white-space: pre;"><div><span style="font-weight: bold; color: #c00;">if</span> (ready) {</div><div>    start()</div><div>}</div></div>',
    'text/plain': 'if (ready) {\n    start()\n}',
  }))
  expect(markdownOf(editor)).toBe('if (ready) {\n    start()\n}')
})

test('pasted plain text is read as the dialect', () => {
  const { editor } = composer()
  paste(editor, transfer({ 'text/plain': 'a **bold** word', 'text/html': '<span>a **bold** word</span>' }))
  expect(markdownOf(editor)).toBe('a **bold** word')
  expect(kinds(editor.getJSON()).has('bold')).toBe(true)
})

test('an attachment dragged from an agent\'s message is the same stored file, uploading nothing', () => {
  const sha256 = 'c'.repeat(64)
  const link = document.createElement('a')
  link.setAttribute('data-attachment-file', JSON.stringify({ type: 'stored', name: 'login.png', sha256, mediaType: 'image/png' }))
  link.innerHTML = '<img src="/blobs/c?type=image%2Fpng" alt="Sign-in">'
  document.body.append(link)
  const dataTransfer = transfer({ 'text/uri-list': 'https://demi.test/blobs/c?type=image%2Fpng' })
  link.addEventListener('dragstart', attachmentDrag)
  link.querySelector('img')!.dispatchEvent(dragEvent('dragstart', dataTransfer))

  const { editor, taken } = composer('See ')
  drop(editor, dataTransfer, 5)
  expect(taken.messages).toEqual([{ message: { text: MARK, files: [{ type: 'stored', name: 'login.png', sha256, mediaType: 'image/png' }] }, picture: null }])
  expect(taken.files).toEqual([])
  expect(kinds(editor.getJSON()).has('image')).toBe(false)
})

test('a capsule dragged within the composer moves, the message keeping its one file', () => {
  const { editor } = composer(`a${MARK} b c`, [{ id: 'f1', name: 'shot.png' }])
  const { view } = editor
  // ProseMirror's record of a drag that began on the capsule, at 2: its
  // typings leave out the node selection a drag of a node keeps.
  Object.assign(view, { dragging: { slice: view.state.doc.slice(2, 3), move: true, node: NodeSelection.create(view.state.doc, 2) } })
  drop(editor, transfer({ 'text/html': '<span data-attachment="{}"></span>' }), 5)
  expect(markdownOf(editor)).toBe(`a b${MARK} c`)
})
