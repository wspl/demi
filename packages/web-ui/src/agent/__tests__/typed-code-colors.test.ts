import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { Editor } from '@tiptap/core'
import { fenceLanguage } from '../../editor/language'
import { resetRegionWaits } from '../../infra/region-wait'
import { codeTextColor, loadHighlighter } from '../../markdown/highlight'
import { codeColors, composerExtensions, typedCodeColors } from '../message-editor/extensions'

// Text a person types, restores from a draft or edits in a sent message is no
// region (`docs/decisions/0033-a-region-draws-once.md`): it shows in its text
// color from the first moment, while the code of a sent message waits hidden
// for its language, and the colors join the typed text as they arrive. One
// editor on a happy-dom page, fake timers, one small language held back:
// under 100 ms.

GlobalRegistrator.register({ url: 'https://demi.test/' })

let open: Editor | null = null
beforeEach(() => {
  jest.useFakeTimers()
  resetRegionWaits()
})
afterEach(() => {
  open?.destroy()
  open = null
  jest.useRealTimers()
})

/** A language no test of the process has loaded yet, whose load waits until `arrive` is called: its fence. */
function gatedLanguage(): { fence: string; arrive: () => Promise<void> } {
  const fence = ['haskell', 'lua', 'erlang', 'ocaml', 'perl', 'pascal', 'fortran', 'scheme'].find((name) => !fenceLanguage(name)?.support)
  const description = fence ? fenceLanguage(fence) : null
  if (!fence || !description)
    throw new Error('every language the test knows is loaded')
  const load = description.load.bind(description)
  let release: () => void = () => {}
  const gate = new Promise<void>((resolve) => {
    release = resolve
  })
  description.load = async () => {
    await gate
    return load()
  }
  return {
    fence,
    arrive: async () => {
      release()
      await description.load()
    },
  }
}

test('code in the composer is never hidden while its language loads, and the colors join it when they arrive', async () => {
  await loadHighlighter()
  const { fence, arrive } = gatedLanguage()
  const node = {
    type: 'doc',
    content: [{ type: 'codeBlock', attrs: { language: fence }, content: [{ type: 'text', text: 'main = putStrLn "hi"' }] }],
  }
  open = new Editor({ element: document.createElement('div'), extensions: composerExtensions({ placeholder: () => '', submit: () => {}, cancel: () => false, editLast: () => false }), content: node })
  const block = open.state.doc.firstChild!
  // The same code in a sent message is a region: hidden while it waits.
  expect(codeColors(block).style).toBe('color: transparent')
  expect(typedCodeColors(block)).toEqual({ style: `color: ${codeTextColor()}`, runs: [] })
  expect(open.view.dom.querySelector('pre')?.getAttribute('style')).not.toContain('transparent')
  jest.advanceTimersByTime(1000)
  expect(typedCodeColors(block).style).toBe(`color: ${codeTextColor()}`)
  await arrive()
  expect(typedCodeColors(block).runs.length).toBeGreaterThan(0)
})
