import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// The composer edits a saved message in the edit mode a sent message is
// edited in: a line at its top says which, its end offers Cancel, Save and
// Send, and Save is also ⌘S (Ctrl+S elsewhere). The draft waits apart. One
// Vue app on a happy-dom page; a few ms besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, h, nextTick } = await import('vue')
const { default: SessionComposer } = await import('../SessionComposer.vue')
const { beginMessageEdit, beginSavedEdit } = await import('../message-editing')
const { provideBlobUrl } = await import('../media-source')
type MessageEditState = import('../message-editing').MessageEditState

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

/** What the composer asked of its page. */
interface Asked {
  saved: number
  sent: string[]
  edits: (MessageEditState | null)[]
}

function mount(edit: MessageEditState): { root: HTMLElement; asked: Asked } {
  const asked: Asked = { saved: 0, sent: [], edits: [] }
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({
    setup: () => provideBlobUrl((source) => `blob:${source}`),
    render: () =>
      h(SessionComposer, {
        draft: 'Draft D',
        messageEdit: edit,
        saved: [
          { id: 'a', preview: 'Message A', attachmentCount: 0, state: 'saved' },
          { id: 'b', preview: 'Message B', attachmentCount: 0, state: 'saved' },
        ],
        providers: [{ id: 'p', label: 'Provider', isAvailable: true }],
        models: { p: [{ id: 'm', name: 'Model', contextWindow: null, contextLimit: null, acceptedExtensions: null, reasoning: null, serviceTiers: null }] },
        upload: () => Promise.reject(new Error('No upload here')),
        claim: async () => null,
        paste: { claim: () => { throw new Error('No paste here') }, remote: () => null },
        onSaveEdit: () => { asked.saved += 1 },
        onSubmitEdit: (way: string) => asked.sent.push(way),
        'onUpdate:messageEdit': (state: MessageEditState | null) => asked.edits.push(state),
      }),
  })
  app.mount(root)
  unmount = () => app.unmount()
  return { root, asked }
}

function button(root: HTMLElement, label: string): HTMLElement | undefined {
  return [...root.querySelectorAll<HTMLElement>('button, [role="button"]')].find((each) => each.getAttribute('aria-label') === label)
}

test('a saved message\'s edit says so, keeps the Saved entry and its count, and Save, ⌘S, Send, Escape and Cancel each ask the page', async () => {
  const { root, asked } = mount(beginSavedEdit('b', [{ type: 'text', text: 'Message B' }]))
  await nextTick()
  expect(root.textContent).toContain('Editing saved message')
  expect(root.textContent).toContain('2 Saved')
  expect(root.querySelector('.ProseMirror')?.textContent).toBe('Message B')
  expect(button(root, 'Save for later')).toBeUndefined()

  button(root, 'Save')!.click()
  root.querySelector('.ProseMirror')!.dispatchEvent(
    new KeyboardEvent('keydown', { key: 's', metaKey: true, ctrlKey: true, bubbles: true, cancelable: true }),
  )
  expect(asked.saved).toBe(2)

  button(root, 'Send')!.click()
  expect(asked.sent).toEqual(['send'])

  root.querySelector('.ProseMirror')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
  button(root, 'Cancel edit')!.click()
  expect(asked.edits).toEqual([null, null])
})

test('a sent message\'s edit says so, and offers no Save', async () => {
  const block = {
    type: 'user' as const, id: 'u1', turnId: 't1', createdAt: '2026-09-09T00:00:00.000Z',
    content: [{ type: 'text' as const, text: 'Check the DNS' }],
  }
  const { root } = mount(beginMessageEdit(block as Parameters<typeof beginMessageEdit>[0], { epoch: 'e', revision: 1 }))
  await nextTick()
  expect(root.textContent).toContain('Editing message')
  expect(root.textContent).not.toContain('Editing saved message')
  expect(button(root, 'Save')).toBeUndefined()
})
