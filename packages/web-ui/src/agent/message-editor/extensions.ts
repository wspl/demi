import { Extension, Node, getSchema, type AnyExtension, type Editor } from '@tiptap/core'
import { Document } from '@tiptap/extension-document'
import { Paragraph } from '@tiptap/extension-paragraph'
import { Text } from '@tiptap/extension-text'
import { Dropcursor, Placeholder, UndoRedo } from '@tiptap/extensions'
import { type Node as ProseMirrorNode, type Schema } from '@tiptap/pm/model'
import { Plugin, PluginKey, type EditorState } from '@tiptap/pm/state'
import { Decoration, DecorationSet } from '@tiptap/pm/view'
import { keepComposition } from '@demicodes/utils'
import { VueNodeViewRenderer } from '@tiptap/vue-3'
import { messageHighlights } from './highlights'
import { lineRoleAt } from './lines'
import { shortcutSet } from '../../ui/shortcut'
import AttachmentNodeView from './AttachmentNodeView.vue'
import { readCapsule } from './capsules'
import { ComposerIntake, type IntakeOptions } from './intake'

// A user message's document (`markdown/user-markdown.ts`), which its editors
// hold and its bubble is drawn from: its text as typed, its highlighting
// (`highlights.ts`) drawn over it, and the editors' keys.

/**
 * A file's capsule, standing where the file is in the text. The node carries
 * the file itself (`capsules.ts` § `MessageCapsule`), so the document says
 * what the message's files are without asking anyone.
 */
const AttachmentNode = Node.create({
  name: 'attachment',
  group: 'inline',
  inline: true,
  atom: true,
  draggable: true,
  addAttributes() {
    return {
      capsule: {
        default: null,
        parseHTML: (element) => readCapsule(element.getAttribute('data-attachment')),
        renderHTML: (attributes) => ({ 'data-attachment': JSON.stringify(attributes['capsule'] ?? null) }),
      },
    }
  },
  parseHTML() {
    return [{ tag: 'span[data-attachment]' }]
  },
  renderHTML({ HTMLAttributes }) {
    return ['span', HTMLAttributes]
  },
  addNodeView() {
    return VueNodeViewRenderer(AttachmentNodeView, { stopEvent: capsuleStopsEvent })
  },
})

/**
 * Which events a capsule keeps from the editor. A drag is the editor's, in a
 * sent message as in the composer, so a capsule can be dragged out of the
 * conversation into a composer: tiptap's own rule lets a node be dragged only
 * after a press in an editable editor. A press selects the capsule and the
 * clipboard's events are the editor's; everything else, Retry's press among it,
 * is the capsule's own.
 */
function capsuleStopsEvent({ event }: { event: Event }): boolean {
  const { type } = event
  if (type.startsWith('drag') || type === 'drop') {
    return false
  }
  const onButton = event.target instanceof Element && !!event.target.closest('button')
  return onButton || !['mousedown', 'copy', 'cut', 'paste'].includes(type)
}

// ── The editors' keys ──

/** Whether the caret is on a fence or in the code between fences. */
export function inCode(state: EditorState): boolean {
  return lineRoleAt(state.doc, state.selection.from) !== 'text'
}

export interface MessageKeyOptions {
  /**
   * Enter outside a code block, or ⌘/Ctrl+Enter anywhere: send. `otherWay`
   * is true for ⌘/Ctrl+Enter outside a code block, which sends the message
   * the other way while the agent works; in a code block ⌘/Ctrl+Enter is the
   * send key, as Enter is elsewhere.
   */
  submit: (otherWay: boolean) => void
  /** Escape: true when it ended something, such as an edit. */
  cancel: () => boolean
  /**
   * Up Arrow in an empty composer: true when it opened something, such as the
   * editor on the last message.
   */
  editLast: () => boolean
}

/**
 * The composer's keys:
 *
 * - Enter sends and Shift+Enter starts a line.
 * - On a fence or in the code between fences, Enter starts a line and
 *   ⌘/Ctrl+Enter sends.
 * - Elsewhere, ⌘/Ctrl+Enter sends the other way.
 * - Up Arrow in a composer with no text and no files may open the editor on the
 *   last message.
 *
 * An input method's keys never reach them (`KeepComposition`).
 */
const MessageKeys = Extension.create<MessageKeyOptions>({
  name: 'messageKeys',
  priority: 1000,
  addOptions() {
    return {
      submit: () => {},
      cancel: () => false,
      editLast: () => false,
    }
  },
  addKeyboardShortcuts() {
    return {
      Enter: ({ editor }) => {
        if (inCode(editor.state)) {
          return editor.commands.splitBlock()
        }
        this.options.submit(false)
        return true
      },
      'Shift-Enter': ({ editor }) => editor.commands.splitBlock(),
      // The keys `otherWayKeys` shows: ⌘ on a Mac and Ctrl on a PC by the
      // page's set, never by the library's own `Mod`.
      [shortcutSet() === 'mac' ? 'Meta-Enter' : 'Ctrl-Enter']: ({ editor }) => {
        this.options.submit(!inCode(editor.state))
        return true
      },
      Escape: () => this.options.cancel(),
      ArrowUp: ({ editor }) => editor.isEmpty && this.options.editLast(),
    }
  },
})

/**
 * An input method's keys are the composition's: the Enter that commits a
 * candidate sends nothing and its Escape ends no edit, in Safari too, where
 * the composition ends before that Enter's keydown. ProseMirror withholds
 * keys from the keymap only while it sees a composition, and in Safari for
 * half a second after one, so the composer keeps them as every text field
 * does, before any keymap and before any listener around the editor.
 */
const KeepComposition = Extension.create({
  name: 'keepComposition',
  // Before every keymap, MessageKeys' among them.
  priority: 10_000,
  addProseMirrorPlugins() {
    return [new Plugin({
      key: new PluginKey('keepComposition'),
      props: {
        // A DOM handler, not `handleKeyDown`, whose true would also cancel
        // the key's default action, the input method's own.
        handleDOMEvents: { keydown: (_view, event) => keepComposition(event) },
      },
    })]
  },
})

// ── What the page draws over the text ──

const decorationsKey = new PluginKey<DecorationSet>('messageDecorations')

/**
 * Asks the editor to draw its decorations again, as when code's colors arrive
 * or the page's mode changes.
 */
export function redrawDecorations(editor: Editor): void {
  editor.view.dispatch(editor.state.tr.setMeta(decorationsKey, true))
}

/**
 * The highlighting over an editor's text (`highlights.ts`): each line's own
 * attributes on its paragraph, each range's around its text.
 */
function decorate(doc: ProseMirrorNode): DecorationSet {
  const decorations: Decoration[] = []
  const lines = messageHighlights(doc)
  doc.forEach((node, offset, index) => {
    const highlight = lines[index]
    if (!highlight) {
      return
    }
    if (highlight.line) {
      decorations.push(Decoration.node(offset, offset + node.nodeSize, highlight.line))
    }
    for (const range of highlight.ranges) {
      decorations.push(Decoration.inline(offset + 1 + range.from, offset + 1 + range.to, range.attributes))
    }
  })
  return DecorationSet.create(doc, decorations)
}

const decorationsPlugin = new Plugin<DecorationSet>({
  key: decorationsKey,
  state: {
    init: (_, state) => decorate(state.doc),
    apply: (tr, previous) => tr.docChanged || tr.getMeta(decorationsKey) ? decorate(tr.doc) : previous,
  },
  props: {
    decorations: (state) => decorationsKey.getState(state),
  },
})

const MessageDecorations = Extension.create({
  name: 'messageDecorations',
  addProseMirrorPlugins() {
    return [decorationsPlugin]
  },
})

let schema: Schema | null = null

/**
 * A message's schema: a paragraph for each line, its text as typed, and a
 * capsule for each file, nothing else, so pasted HTML keeps only its text and
 * its files. The editors
 * hold it, and a sent message is drawn from it (`SentMessage`).
 */
export function messageSchema(): Schema {
  schema ??= getSchema(messageExtensions())
  return schema
}

function messageExtensions(): AnyExtension[] {
  return [Document, Paragraph, Text, AttachmentNode]
}

/**
 * A message editor: the message's schema, its highlighting drawn over the
 * text, the composer's keys, and what it takes in from pastes and drops.
 */
export function composerExtensions(
  options: MessageKeyOptions & { placeholder: () => string; intake?: IntakeOptions },
): AnyExtension[] {
  return [
    ...messageExtensions(),
    MessageDecorations,
    ...(options.intake ? [ComposerIntake.configure(options.intake)] : [ComposerIntake]),
    KeepComposition,
    MessageKeys.configure({ submit: options.submit, cancel: options.cancel, editLast: options.editLast }),
    Placeholder.configure({ placeholder: options.placeholder }),
    UndoRedo,
    Dropcursor.configure({ class: 'message-drop-cursor', color: false, width: 2 }),
  ]
}
