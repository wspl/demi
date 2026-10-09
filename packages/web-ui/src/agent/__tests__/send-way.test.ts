import { expect, test } from 'bun:test'
import { Editor, isMacOS, type JSONContent } from '@tiptap/core'
import type { EditorView } from '@tiptap/pm/view'
import { composerExtensions } from '../message-editor/extensions'
import { chooseSendWay, type SendWay, type SendWhileRunning } from '../send-way'

// The composer's keys reach its real keymap, and what they ask goes through
// the composer's choice of way (`product.md` § Steer or queue). About 30 ms.

const PARAGRAPH: JSONContent = { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'skip the e2e tests' }] }] }
const CODE: JSONContent = { type: 'doc', content: [{ type: 'codeBlock', content: [{ type: 'text', text: 'bun test' }] }] }

/**
 * Presses Enter, with ⌘ on macOS or Ctrl elsewhere when `mod`, in a composer
 * holding `content`; returns whether it sent and whether it asked for the other way.
 */
function press(content: JSONContent, mod: boolean): boolean[] {
  const sent: boolean[] = []
  const editor = new Editor({
    element: null,
    extensions: composerExtensions({ placeholder: '', submit: (otherWay) => sent.push(otherWay), cancel: () => false, editLast: () => false }),
    content,
  })
  const event = Object.assign(new Event('keydown'), {
    key: 'Enter',
    keyCode: 13,
    metaKey: mod && isMacOS(),
    ctrlKey: mod && !isMacOS(),
    altKey: false,
    shiftKey: false,
  })
  // ProseMirror's keymap reads only the view's state and the key with its
  // modifiers; a test has no DOM to mount an EditorView in, so the editor's
  // own plugins get a view of its state and an event of those fields.
  const view = { state: editor.state } as EditorView
  for (const plugin of editor.extensionManager.plugins) {
    if (plugin.props.handleKeyDown?.call(plugin, view, event as KeyboardEvent)) {
      break
    }
  }
  editor.destroy()
  return sent
}

function way(sent: boolean[], working: boolean, preferred: SendWhileRunning): SendWay[] {
  return sent.map((otherWay) => chooseSendWay(working, preferred, otherWay))
}

test('Enter sends the preferred way while the agent works, ⌘/Ctrl+Enter the other way, and both just send while it is idle', () => {
  for (const preferred of ['steer', 'queue'] as const) {
    const other = preferred === 'steer' ? 'queue' : 'steer'
    expect(way(press(PARAGRAPH, false), true, preferred)).toEqual([preferred])
    expect(way(press(PARAGRAPH, true), true, preferred)).toEqual([other])
    expect(way(press(PARAGRAPH, false), false, preferred)).toEqual(['send'])
    expect(way(press(PARAGRAPH, true), false, preferred)).toEqual(['send'])
  }
})

test('in a code block ⌘/Ctrl+Enter is the send key and sends the preferred way', () => {
  expect(way(press(CODE, true), true, 'steer')).toEqual(['steer'])
  expect(way(press(CODE, true), true, 'queue')).toEqual(['queue'])
  expect(way(press(CODE, true), false, 'queue')).toEqual(['send'])
})
