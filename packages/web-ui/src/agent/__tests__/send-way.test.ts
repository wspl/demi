import { expect, test } from 'bun:test'
import { Editor, isMacOS, type JSONContent } from '@tiptap/core'
import type { EditorView } from '@tiptap/pm/view'
import { composerExtensions } from '../message-editor/extensions'
import { chooseSendWay, type SendWay, type SendWhileRunning } from '../send-way'

// The composer's keys reach its real keymap, and what they ask goes through
// the composer's choice of way (`product.md` § Steer or queue); an input
// method's keys ask nothing. About 30 ms.

const PARAGRAPH: JSONContent = { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'skip the e2e tests' }] }] }
const CODE: JSONContent = { type: 'doc', content: [{ type: 'codeBlock', content: [{ type: 'text', text: 'bun test' }] }] }

/** A key as the keydown event gives it; `mod` is ⌘ on macOS and Ctrl elsewhere. */
interface Key {
  key: string
  keyCode: number
  isComposing?: boolean
  mod?: boolean
}

const ENTER: Key = { key: 'Enter', keyCode: 13 }
const MOD_ENTER: Key = { ...ENTER, mod: true }

/** What a keydown in a composer did: the sends it asked, the edits it ended, and whether listeners around the editor got it. */
interface Pressed {
  sent: boolean[]
  cancelled: number
  reachesPage: boolean
}

/** Presses `key` in a composer holding `content`. */
function press(content: JSONContent, key: Key): Pressed {
  const pressed: Pressed = { sent: [], cancelled: 0, reachesPage: true }
  const editor = new Editor({
    element: null,
    extensions: composerExtensions({
      placeholder: '',
      submit: (otherWay) => pressed.sent.push(otherWay),
      cancel: () => {
        pressed.cancelled += 1
        return true
      },
      editLast: () => false,
    }),
    content,
  })
  const event = Object.assign(new Event('keydown'), {
    key: key.key,
    keyCode: key.keyCode,
    isComposing: key.isComposing ?? false,
    metaKey: !!key.mod && isMacOS(),
    ctrlKey: !!key.mod && !isMacOS(),
    altKey: false,
    shiftKey: false,
    stopPropagation: () => {
      pressed.reachesPage = false
    },
  }) as KeyboardEvent
  // ProseMirror gives a keydown to the plugins' DOM handlers first and to
  // the keymaps only when none took it; both read only the view's state and
  // the key's fields. A test has no DOM to mount an EditorView in, so the
  // editor's own plugins get a view of its state and an event of those fields.
  const view = { state: editor.state } as EditorView
  const plugins = editor.extensionManager.plugins
  const taken = plugins.some((plugin) => plugin.props.handleDOMEvents?.keydown?.call(plugin, view, event))
  if (!taken) {
    plugins.some((plugin) => plugin.props.handleKeyDown?.call(plugin, view, event))
  }
  editor.destroy()
  return pressed
}

function way(pressed: Pressed, working: boolean, preferred: SendWhileRunning): SendWay[] {
  return pressed.sent.map((otherWay) => chooseSendWay(working, preferred, otherWay))
}

test('Enter sends the preferred way while the agent works, ⌘/Ctrl+Enter the other way, and both just send while it is idle', () => {
  for (const preferred of ['steer', 'queue'] as const) {
    const other = preferred === 'steer' ? 'queue' : 'steer'
    expect(way(press(PARAGRAPH, ENTER), true, preferred)).toEqual([preferred])
    expect(way(press(PARAGRAPH, MOD_ENTER), true, preferred)).toEqual([other])
    expect(way(press(PARAGRAPH, ENTER), false, preferred)).toEqual(['send'])
    expect(way(press(PARAGRAPH, MOD_ENTER), false, preferred)).toEqual(['send'])
  }
})

test('in a code block ⌘/Ctrl+Enter is the send key and sends the preferred way', () => {
  expect(way(press(CODE, MOD_ENTER), true, 'steer')).toEqual(['steer'])
  expect(way(press(CODE, MOD_ENTER), true, 'queue')).toEqual(['queue'])
  expect(way(press(CODE, MOD_ENTER), false, 'queue')).toEqual(['send'])
})

test("an input method's Enter sends nothing and its Escape ends no edit, in Chrome's order and in Safari's", () => {
  const kept: Pressed = { sent: [], cancelled: 0, reachesPage: false }
  // Chrome: the Enter that commits the candidate comes while the composition is open.
  expect(press(PARAGRAPH, { key: 'Enter', keyCode: 229, isComposing: true })).toEqual(kept)
  // Safari: the composition ended before the Enter's keydown, which only its key code marks.
  expect(press(PARAGRAPH, { key: 'Enter', keyCode: 229 })).toEqual(kept)
  expect(press(PARAGRAPH, { key: 'Escape', keyCode: 229 })).toEqual(kept)
  // The next Enter is the user's own.
  expect(press(PARAGRAPH, ENTER)).toEqual({ sent: [false], cancelled: 0, reachesPage: true })
  expect(press(PARAGRAPH, { key: 'Escape', keyCode: 27 })).toEqual({ sent: [], cancelled: 1, reachesPage: true })
})
