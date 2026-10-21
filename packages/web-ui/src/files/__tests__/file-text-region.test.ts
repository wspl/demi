import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, jest, mock, test } from 'bun:test'

// A file's text in the file view is a region (`docs/decisions/0033-a-region-draws-once.md`):
// while the editor loads it draws nothing for 300 ms at most, then the text
// without colors, which the editor replaces once it is built, never a loading
// note in between. One Vue app on a happy-dom page, a stand-in editor, fake
// timers; a few ms.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, shallowRef } = await import('vue')
const { resetRegionWaits } = await import('../../infra/region-wait')

/** The editor's chunk: on its way, or there. */
const editor = shallowRef<{ phase: 'loading' } | { phase: 'ready'; chunk: { languageName: () => string | null } }>({ phase: 'loading' })
/** What the stand-in editor tells the view: it is built. */
let built: (() => void) | null = null
void mock.module('@demicodes/web-ui/editor/load', () => ({
  useEditor: () => ({ load: editor, retry: () => {} }),
  CodeEditor: defineComponent({
    props: { path: { type: String, required: true }, text: { type: String, required: true } },
    emits: ['built', 'left'],
    setup(props, { emit }) {
      built = () => emit('built', props.path)
      return () => h('div', { 'data-editor': '' }, props.text)
    },
  }),
  DiffEditor: defineComponent({ render: () => h('div') }),
  reportSyntaxColorsUnavailable: () => {},
}))
const { default: FileContent } = await import('../FileContent.vue')
const { createMemoryFileSource, dir, file } = await import('../memory-source')

let unmount: (() => void) | null = null
beforeEach(() => {
  jest.useFakeTimers()
  resetRegionWaits()
  editor.value = { phase: 'loading' }
  built = null
})
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
  jest.useRealTimers()
})

async function mountFile(): Promise<HTMLElement> {
  const source = createMemoryFileSource({
    platform: 'linux',
    home: '/home/alex',
    root: dir({ home: dir({ alex: dir({ 'notes.txt': file(11, '2026-09-02T10:00:00Z', 'first line\nsecond') }) }) }),
  })
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({ render: () => h(FileContent, { source, root: '/home/alex', path: '/home/alex/notes.txt' }) })
  app.mount(root)
  unmount = () => app.unmount()
  // The file's text is read.
  for (let step = 0; step < 20 && !root.textContent?.includes('first line') && !root.querySelector('[data-editor]'); step++) {
    await nextTick()
    await Promise.resolve()
    if (editor.value.phase === 'ready')
      break
  }
  return root
}

test('the text waits blank for 300 ms, with no loading note, then shows without colors until the editor is built', async () => {
  const root = await mountFile()
  await nextTick()
  expect(root.querySelector('[data-plain-text]')).toBeNull()
  expect(root.textContent).not.toContain('first line')
  expect(root.textContent).not.toContain('Reading')
  // The bar stands at once, with the facts and not yet the language, so the text does not move when the editor arrives.
  expect(root.textContent).toContain('2 lines')
  expect(root.textContent).not.toContain('Plain Text')

  jest.advanceTimersByTime(300)
  await nextTick()
  expect(root.querySelector('[data-plain-text]')?.textContent).toBe('first line\nsecond')

  // The editor's chunk arrives and is built: the colors replace the plain text in its place.
  editor.value = { phase: 'ready', chunk: { languageName: () => 'Plain Text' } }
  await nextTick()
  expect(root.querySelector('[data-plain-text]')).not.toBeNull()
  expect(root.textContent).toContain('Plain Text · 2 lines')
  built?.()
  await nextTick()
  expect(root.querySelector('[data-plain-text]')).toBeNull()
  expect(root.querySelector('[data-editor]')?.textContent).toBe('first line\nsecond')
})

test('an editor built within 300 ms draws once, with no plain text', async () => {
  const root = await mountFile()
  jest.advanceTimersByTime(200)
  editor.value = { phase: 'ready', chunk: { languageName: () => 'Plain Text' } }
  await nextTick()
  built?.()
  await nextTick()
  jest.advanceTimersByTime(300)
  await nextTick()
  expect(root.querySelector('[data-plain-text]')).toBeNull()
  expect(root.querySelector('[data-editor]')).not.toBeNull()
})
