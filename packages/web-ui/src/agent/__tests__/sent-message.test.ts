import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A sent message drawn without an editor (`composer.md` § Writing a
// message): it shows what the composer's editor showed, and a copy of all
// or a part of it carries its Markdown with its files, as the editor's copy
// did. One Vue app on a happy-dom page; a few ms besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick } = await import('vue')
const { default: SentMessage } = await import('../message-editor/SentMessage.vue')
const { provideBlobUrl } = await import('../media-source')
const { ATTACHMENT_MARK: MARK } = await import('../../markdown/user-markdown')
const { COPIED_MESSAGE_ATTRIBUTE, messageClipboard, readCopiedMessage } = await import('../message-input/copied-message')
const { parseUserMarkdown } = await import('../../markdown/user-markdown')

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

/** A file the message holds as the user's blob, which a copy carries without its bytes. */
const notes = {
  id: 'notes',
  name: 'notes.md',
  path: '/home/demi/.demi/attachments/c/notes.md',
  reference: { type: 'stored' as const, sha256: 'a'.repeat(64), mediaType: 'text/markdown', size: 18 },
}

async function mount(markdown: string): Promise<HTMLElement> {
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp(defineComponent({
    setup() {
      provideBlobUrl((ref) => `blob:${ref}`)
      return () => h(SentMessage, { markdown, attachments: [notes] })
    },
  }))
  app.mount(root)
  unmount = () => app.unmount()
  await nextTick()
  return root.querySelector<HTMLElement>('.sent-message')!
}

/** Copies what `range` selects, as ⌘C does: the forms the page put on the clipboard, or none when it left the copy to the browser. */
function copyOf(range: Range): { plain: string; html: string } | null {
  const selection = document.getSelection()!
  selection.removeAllRanges()
  selection.addRange(range)
  const clipboardData = new DataTransfer()
  const event = new ClipboardEvent('copy', { clipboardData, bubbles: true, cancelable: true })
  const start = range.startContainer
  ;(start instanceof Element ? start : start.parentElement!).dispatchEvent(event)
  return event.defaultPrevented ? { plain: clipboardData.getData('text/plain'), html: clipboardData.getData('text/html') } : null
}

const MESSAGE = `Compare **before** with https://example.com/a, notes in ${MARK}\n\n\`\`\`ts\nconst a = 1\n\`\`\``

test('a sent message shows its formatting, its bare links and each file as a capsule', async () => {
  const message = await mount(MESSAGE)
  expect(message.querySelector('strong')?.textContent).toBe('before')
  expect(message.querySelector('a[data-link="web"]')?.getAttribute('href')).toBe('https://example.com/a')
  expect(message.querySelectorAll('[data-attachment]')).toHaveLength(1)
  expect(message.querySelector('pre code')?.textContent).toBe('const a = 1')
})

test('a copy of a sent message, whole or in part, is its Markdown with its files; a part of its code is the code', async () => {
  const message = await mount(MESSAGE)
  const whole = document.createRange()
  whole.selectNodeContents(message)
  const all = copyOf(whole)
  expect(all?.plain).toBe('Compare **before** with https://example.com/a, notes in notes.md\n\n```ts\nconst a = 1\n```')
  // Demi's own form holds the message as the message itself says it, its file with it.
  const html = new DOMParser().parseFromString(all?.html ?? '', 'text/html')
  const form = html.querySelector(`[${COPIED_MESSAGE_ATTRIBUTE}]`)?.getAttribute(COPIED_MESSAGE_ATTRIBUTE) ?? ''
  expect(readCopiedMessage(form)).toEqual(messageClipboard(parseUserMarkdown(MESSAGE, [notes])).message)
  expect(readCopiedMessage(form)?.files).toEqual([{ type: 'stored', name: 'notes.md', sha256: 'a'.repeat(64), mediaType: 'text/markdown', size: 18 }])

  const part = document.createRange()
  const line = message.querySelector('p')!
  part.setStart([...line.childNodes].find((node) => node.textContent?.startsWith('Compare'))!, 3)
  part.setEnd(line, line.childNodes.length)
  expect(copyOf(part)?.plain).toBe('pare **before** with https://example.com/a, notes in notes.md')

  const code = document.createRange()
  code.selectNodeContents(message.querySelector('pre code')!)
  expect(copyOf(code)).toBeNull()
})

test('a copy or a drag of a part wholly within bold, inline code or a link keeps its formatting', async () => {
  const message = await mount('Say **before**, `code` and [the site](https://example.com/a)')
  const within = (element: Element): Range => {
    const text = element.firstChild!
    const range = document.createRange()
    range.setStart(text, 1)
    range.setEnd(text, text.textContent!.length - 1)
    return range
  }
  expect(copyOf(within(message.querySelector('strong')!))?.plain).toBe('**efor**')
  expect(copyOf(within(message.querySelector('code')!))?.plain).toBe('`od`')
  expect(copyOf(within(message.querySelector('a')!))?.plain).toBe('[he sit](https://example.com/a)')

  // A drag of the same part carries the same.
  const selection = document.getSelection()!
  selection.removeAllRanges()
  selection.addRange(within(message.querySelector('strong')!))
  const dataTransfer = new DataTransfer()
  const drag = new DragEvent('dragstart', { bubbles: true, cancelable: true })
  // happy-dom's DragEvent leaves out the `dataTransfer` its init names; a browser's carries it.
  Object.defineProperty(drag, 'dataTransfer', { value: dataTransfer })
  message.querySelector('strong')!.dispatchEvent(drag)
  expect(dataTransfer.getData('text/plain')).toBe('**efor**')
})
