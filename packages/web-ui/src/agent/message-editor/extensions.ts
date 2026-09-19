import { Extension, Mark, Node, type AnyExtension, type Editor, type JSONContent } from '@tiptap/core'
import { Bold } from '@tiptap/extension-bold'
import { Code } from '@tiptap/extension-code'
import { CodeBlock } from '@tiptap/extension-code-block'
import { Document } from '@tiptap/extension-document'
import { HardBreak } from '@tiptap/extension-hard-break'
import { Italic } from '@tiptap/extension-italic'
import { Paragraph } from '@tiptap/extension-paragraph'
import { Strike } from '@tiptap/extension-strike'
import { Text } from '@tiptap/extension-text'
import { Dropcursor, Placeholder, UndoRedo } from '@tiptap/extensions'
import { Fragment, type Node as ProseMirrorNode, type ResolvedPos } from '@tiptap/pm/model'
import { Plugin, PluginKey, TextSelection, type EditorState, type Transaction } from '@tiptap/pm/state'
import { Decoration, DecorationSet, type EditorView } from '@tiptap/pm/view'
import { VueNodeViewRenderer } from '@tiptap/vue-3'
import { isHttpUrl, isLikelyFilePath } from '../../markdown/filePath'
import { codeStyles } from '../../markdown/highlight'
import { bareUrls, closesFence, closingFormat, fenceInfo } from '../../markdown/user-markdown'
import AttachmentNodeView from './AttachmentNodeView.vue'
import ImageNodeView from './ImageNodeView.vue'

// The editor a user message is written and shown in (`product.md` § Writing
// a message): the schema the dialect reads into, formatting applied as a
// construct is typed, the composer's keys, and what the page draws over the
// text (links for bare URLs, colors for code).

/** A file's capsule, standing where the file is in the text. */
const AttachmentNode = Node.create({
  name: 'attachment',
  group: 'inline',
  inline: true,
  atom: true,
  draggable: true,
  addAttributes() {
    return {
      id: {
        default: '',
        parseHTML: (element) => element.getAttribute('data-attachment') ?? '',
        renderHTML: (attributes) => ({ 'data-attachment': String(attributes['id']) }),
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
    return VueNodeViewRenderer(AttachmentNodeView)
  },
})

/** `![alt](target)`: the image, loaded as `file-previews.md` § Files named in messages says. */
const ImageNode = Node.create({
  name: 'image',
  group: 'inline',
  inline: true,
  atom: true,
  draggable: true,
  addAttributes() {
    return {
      src: { default: '' },
      alt: { default: '' },
      title: { default: null },
    }
  },
  parseHTML() {
    return [{ tag: 'img[src]' }]
  },
  renderHTML({ HTMLAttributes }) {
    return ['img', HTMLAttributes]
  },
  addNodeView() {
    return VueNodeViewRenderer(ImageNodeView)
  },
})

/**
 * How a link or a bare URL in a message opens: a web address in a new tab, a
 * Host file in the File view, and anything else not at all, shown as text.
 */
export function linkAttributes(target: string): Record<string, string> {
  if (isHttpUrl(target)) {
    return { 'data-link': 'web', 'data-target': target, href: target, target: '_blank', rel: 'noopener noreferrer' }
  }
  return { 'data-link': isLikelyFilePath(target) ? 'file' : 'text', 'data-target': target }
}

/** `[text](target)`. Typing at its end continues plain text. */
const LinkMark = Mark.create({
  name: 'link',
  inclusive: false,
  addAttributes() {
    return {
      href: {
        default: '',
        parseHTML: (element) => element.getAttribute('data-target') ?? element.getAttribute('href') ?? '',
        rendered: false,
      },
      title: { default: null },
    }
  },
  parseHTML() {
    return [{ tag: 'a[data-target]' }, { tag: 'a[href]' }]
  },
  renderHTML({ mark, HTMLAttributes }) {
    return ['a', { ...HTMLAttributes, ...linkAttributes(String(mark.attrs['href'])) }, 0]
  },
})

// ── Formatting as it is typed ──

/** The characters a conversion replaced, which Backspace right after gives back. */
interface FormatUndo {
  transform: Transaction
  from: number
  to: number
  text: string
}

const formatKey = new PluginKey<FormatUndo | null>('messageFormat')

/** The inline run that ends at `$pos` on its line, split into what comes before its plain tail and that tail. */
function runBefore($pos: ResolvedPos): { before: JSONContent[]; tail: string } {
  let run: ProseMirrorNode[] = []
  $pos.parent.forEach((child, offset) => {
    if (offset >= $pos.parentOffset) {
      return
    }
    if (child.type.name === 'hardBreak' || child.type.name === 'attachment') {
      run = []
      return
    }
    run.push(offset + child.nodeSize > $pos.parentOffset ? child.cut(0, $pos.parentOffset - offset) : child)
  })
  const last = run.at(-1)
  const tail = last?.isText && !last.marks.length ? last.text ?? '' : ''
  return {
    before: (tail ? run.slice(0, -1) : run).map((node) => node.toJSON()),
    tail,
  }
}

/**
 * Types `text` at `from`–`to` and, when it closes a construct of the dialect,
 * formats it in the same step; true when it did.
 */
function formatTyped(view: EditorView, from: number, to: number, text: string): boolean {
  const { state } = view
  const $from = state.doc.resolve(from)
  if (from !== to || !$from.parent.isTextblock || $from.parent.type.spec.code) {
    return false
  }
  if ((state.storedMarks ?? $from.marks()).some((mark) => mark.type.spec.code)) {
    return false
  }
  const { before, tail } = runBefore($from)
  const closed = closingFormat(before, tail + text)
  if (!closed) {
    return false
  }
  const tr = state.tr.insertText(text, from, to)
  const end = from + text.length
  tr.replaceWith(end - closed.length, end, closed.nodes.map((node) => state.schema.nodeFromJSON(node)))
  tr.setStoredMarks([])
  tr.setMeta(formatKey, { transform: tr, from, to, text } satisfies FormatUndo)
  view.dispatch(tr)
  return true
}

const formatPlugin = new Plugin<FormatUndo | null>({
  key: formatKey,
  // tiptap's Backspace gives back the characters of the last conversion a
  // plugin marked this way records.
  isInputRules: true,
  state: {
    init: () => null,
    apply(tr, previous) {
      const recorded: FormatUndo | undefined = tr.getMeta(formatKey)
      if (recorded) {
        return recorded
      }
      return tr.selectionSet || tr.docChanged ? null : previous
    },
  },
  props: {
    handleTextInput: formatTyped,
    handleDOMEvents: {
      compositionend(view) {
        // An input method's text lands after this event.
        setTimeout(() => {
          const { selection } = view.state
          const cursor = selection instanceof TextSelection ? selection.$cursor : null
          if (!view.isDestroyed && cursor) {
            formatTyped(view, cursor.pos, cursor.pos, '')
          }
        })
        return false
      },
    },
  },
})

/** The text of the line the cursor ends: from the line break before it, or the block's start. */
function lineAt($pos: ResolvedPos): { from: number; text: string; plain: boolean; breakBefore: boolean } | null {
  let from = $pos.start()
  let text = ''
  let plain = true
  let breakBefore = false
  $pos.parent.forEach((child, offset) => {
    if (offset >= $pos.parentOffset) {
      return
    }
    if (child.type.name === 'hardBreak') {
      from = $pos.start() + offset + child.nodeSize
      text = ''
      plain = true
      breakBefore = true
      return
    }
    // Only a line of plain text can be a fence.
    text += child.text ?? ''
    plain &&= child.isText && !child.marks.length
  })
  const after = $pos.parent.childAfter($pos.parentOffset).node
  return after && after.type.name !== 'hardBreak' ? null : { from, text, plain, breakBefore }
}

/**
 * A line typed as a fence opens a code block when it ends: the paragraph
 * splits around it, and the cursor moves into the block.
 */
function openFence(editor: Editor): boolean {
  const { state, view } = editor
  const { $from, empty } = state.selection
  if (!empty || $from.parent.type.name !== 'paragraph') {
    return false
  }
  const line = lineAt($from)
  const info = line?.plain ? fenceInfo(line.text) : null
  if (!line || info === null) {
    return false
  }
  const { paragraph, codeBlock } = state.schema.nodes
  const start = $from.start()
  const beforeEnd = line.from - start - (line.breakBefore ? 1 : 0)
  const afterContent = $from.parent.content.cut($from.parentOffset)
  const afterStart = afterContent.firstChild?.type.name === 'hardBreak' ? afterContent.cut(1) : afterContent
  const blocks: ProseMirrorNode[] = []
  if (line.breakBefore) {
    blocks.push(paragraph!.create(null, $from.parent.content.cut(0, beforeEnd)))
  }
  blocks.push(codeBlock!.create({ language: info || null }))
  if (afterContent.size) {
    blocks.push(paragraph!.create(null, afterStart))
  }
  const tr = state.tr.replaceWith($from.before(), $from.after(), Fragment.from(blocks))
  const blockStart = $from.before() + (line.breakBefore ? blocks[0]!.nodeSize : 0)
  tr.setSelection(TextSelection.create(tr.doc, blockStart + 1))
  tr.setMeta(formatKey, { transform: tr, from: $from.pos, to: $from.pos, text: '' } satisfies FormatUndo)
  view.dispatch(tr.scrollIntoView())
  return true
}

/** A fence typed as a code block's last line closes it: the cursor leaves for a line after the block. */
function closeFence(editor: Editor): boolean {
  const { state, view } = editor
  const { $from, empty } = state.selection
  const code = $from.parent
  if (!empty || !code.type.spec.code || $from.parentOffset !== code.content.size) {
    return false
  }
  const text = code.textContent
  const lineStart = text.lastIndexOf('\n') + 1
  if (!closesFence(text.slice(lineStart))) {
    return false
  }
  const tr = state.tr.delete($from.start() + Math.max(0, lineStart - 1), $from.pos)
  const after = tr.mapping.map($from.after())
  tr.insert(after, state.schema.nodes['paragraph']!.create())
  tr.setSelection(TextSelection.create(tr.doc, after + 1))
  tr.setMeta(formatKey, { transform: tr, from: $from.pos, to: $from.pos, text: '' } satisfies FormatUndo)
  view.dispatch(tr.scrollIntoView())
  return true
}

function inCode(state: EditorState): boolean {
  return !!state.selection.$from.parent.type.spec.code
}

export interface MessageKeyOptions {
  /** Enter outside a code block, or ⌘/Ctrl+Enter anywhere: send. */
  submit: () => void
  /** Escape: true when it ended something, such as an edit. */
  cancel: () => boolean
}

/**
 * The composer's keys: Enter sends and Shift+Enter breaks the line; in a
 * code block Enter breaks the line and ⌘/Ctrl+Enter sends. A line typed as a
 * fence opens or closes a code block when it ends.
 */
const MessageKeys = Extension.create<MessageKeyOptions>({
  name: 'messageKeys',
  priority: 1000,
  addOptions() {
    return {
      submit: () => {},
      cancel: () => false,
    }
  },
  addKeyboardShortcuts() {
    const lineEnd = (editor: Editor) =>
      closeFence(editor) || openFence(editor) || (inCode(editor.state) && editor.commands.newlineInCode())
    return {
      Enter: ({ editor }) => {
        if (lineEnd(editor)) {
          return true
        }
        this.options.submit()
        return true
      },
      'Shift-Enter': ({ editor }) => lineEnd(editor) || editor.commands.setHardBreak(),
      'Mod-Enter': () => {
        this.options.submit()
        return true
      },
      Escape: () => this.options.cancel(),
    }
  },
})

// ── What the page draws over the text ──

const decorationsKey = new PluginKey<DecorationSet>('messageDecorations')

/** Asks the editor to draw its decorations again, as when code's colors arrive or the page's mode changes. */
export function redrawDecorations(editor: Editor): void {
  editor.view.dispatch(editor.state.tr.setMeta(decorationsKey, true))
}

function decorate(doc: ProseMirrorNode): DecorationSet {
  const decorations: Decoration[] = []
  doc.descendants((node, pos) => {
    if (node.type.spec.code) {
      const language = String(node.attrs['language'] ?? '').split(/\s/)[0] ?? ''
      for (const run of codeStyles(node.textContent, language) ?? []) {
        decorations.push(Decoration.inline(pos + 1 + run.from, pos + 1 + run.to, { style: run.style }))
      }
      return false
    }
    // A bare URL is a link in the text itself, not in a code or a written link.
    if (node.isText && node.text?.includes('http') && !node.marks.some((mark) => mark.type.name === 'link' || mark.type.spec.code)) {
      for (const url of bareUrls(node.text)) {
        decorations.push(Decoration.inline(pos + url.from, pos + url.to, { nodeName: 'a', ...linkAttributes(url.href) }))
      }
    }
    return true
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

const MessageFormat = Extension.create({
  name: 'messageFormat',
  addProseMirrorPlugins() {
    return [formatPlugin]
  },
})

/** A message as the conversation shows it: what it holds and how it looks, one schema with the composer's. */
export function readOnlyExtensions(): AnyExtension[] {
  return [
    Document,
    Paragraph,
    Text,
    // A new line starts plain, as each line of the dialect does.
    HardBreak.configure({ keepMarks: false }),
    Bold,
    Italic,
    Strike,
    // Code sits inside bold or a link: `**`a`**`, [`a`](b).
    Code.extend({ excludes: 'code' }),
    CodeBlock,
    LinkMark,
    ImageNode,
    AttachmentNode,
    MessageDecorations,
  ]
}

/** The composer: the same message, with formatting as it is typed and the composer's keys. */
export function composerExtensions(options: MessageKeyOptions & { placeholder: string }): AnyExtension[] {
  return [
    ...readOnlyExtensions(),
    MessageFormat,
    MessageKeys.configure({ submit: options.submit, cancel: options.cancel }),
    Placeholder.configure({ placeholder: options.placeholder }),
    UndoRedo,
    Dropcursor.configure({ class: 'message-drop-cursor', color: false, width: 2 }),
  ]
}
