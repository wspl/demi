<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, watch } from 'vue'
import { Fragment, Slice, type Node as ProseMirrorNode, type Schema } from '@tiptap/pm/model'
import { Selection, TextSelection, type Transaction } from '@tiptap/pm/state'
import type { EditorView } from '@tiptap/pm/view'
import { Editor, EditorContent } from '@tiptap/vue-3'
import { transferHasFiles } from '../../composables/useFileDrop'
import { messageHostPath } from '../../markdown/filePath'
import { useMarkdownRenderVersion } from '../../markdown/highlight'
import { useMessageFiles } from '../../markdown/message-files'
import { parseUserMarkdown, serializeUserMarkdown } from '../../markdown/user-markdown'
import { useAutofocus } from '../../ui/autofocus'
import { dataTransferFiles, pastedTextFile, pastedTextIsLong } from '../message-input/attachments'
import { useTransfers, type MessageCapsule } from './capsules'
import { composerExtensions, readOnlyExtensions, redrawDecorations } from './extensions'

/**
 * A user message, written or shown in one editor (`product.md` § Writing a
 * message): its Markdown formatted as it will look, each file a capsule where
 * it stands. In the composer it is typed into; in the conversation it is the
 * sent message, read-only.
 */
const props = withDefaults(
  defineProps<{
    /** The message's Markdown, an attachment mark where each capsule stands. */
    markdown: string
    /** Its files, in the order of their marks; the document keeps them from there on. */
    attachments?: readonly MessageCapsule[]
    /** The composer's editor: formatting as it is typed, Enter to send, files dropped and pasted. */
    composer?: boolean
    /** A composer that takes no typing for now, such as an edit being sent. */
    disabled?: boolean
    /** Escape ends something, such as an edit. */
    cancelable?: boolean
    placeholder?: string
    /** What assistive technology calls the field. */
    label?: string
    /**
     * How much width one line of the composer offers the text (`ComposerShell`
     * measures it): a message wider than that needs more than one line.
     */
    lineWidth?: number
    /** Takes the focus once it shows, the cursor at the end. */
    autofocus?: boolean
  }>(),
  {
    attachments: () => [],
    placeholder: '',
  },
)

const emit = defineEmits<{
  /** The message changed: its Markdown, and the files of its capsules in the order of their marks. */
  change: [markdown: string, attachments: MessageCapsule[]]
  submit: []
  cancel: []
  /** Files pasted, or a paste long enough to be one; their capsules land where the cursor was. */
  files: [files: File[]]
  focus: []
  blur: []
}>()

/**
 * The message needs more than one line: it holds lines of its own (a line
 * break, a code block or an image), or its text is wider than a line.
 */
const multiline = defineModel<boolean>('multiline', { default: false })

const files = useMessageFiles()
const autofocus = useAutofocus()
const editable = computed(() => !!props.composer && !props.disabled)
const transfers = useTransfers()

/** The Markdown the editor last showed or wrote, so the same text coming back as a prop is not shown again. */
let shown = props.markdown
/** Where the next new capsules land: where files were dropped, or where the cursor was. */
let landing: number | null = null

const editor = new Editor({
  extensions: props.composer
    ? composerExtensions({
        placeholder: props.placeholder,
        submit: () => emit('submit'),
        cancel: () => {
          if (props.cancelable) {
            emit('cancel')
          }
          return !!props.cancelable
        },
      })
    : readOnlyExtensions(),
  content: parseUserMarkdown(props.markdown, props.attachments),
  editable: editable.value,
  // Formatting as it is typed and pasted text follow the dialect, not tiptap's own rules.
  enableInputRules: false,
  enablePasteRules: false,
  editorProps: {
    attributes: {
      class: 'markdown-body message-editor-text',
      ...(props.label ? { 'aria-label': props.label } : {}),
    },
    handlePaste: (view, event) => props.composer && paste(view, event),
    handleDrop: (view, event, _slice, moved) => props.composer && drop(view, event, moved),
    transformPasted: (slice, view) => new Slice(keepCapsules(slice.content, view), slice.openStart, slice.openEnd),
    clipboardTextSerializer: (slice) => {
      const parts: string[] = []
      slice.content.forEach((node) => parts.push(plainText(node)))
      return parts.join(slice.content.firstChild?.isBlock ? '\n' : '')
    },
  },
  onUpdate: write,
  onTransaction: ({ transaction }) => {
    if (landing !== null) {
      landing = transaction.mapping.map(landing)
    }
  },
  onFocus: () => emit('focus'),
  onBlur: () => emit('blur'),
})
// The cursor starts at the end, where files picked before a click land.
editor.view.dispatch(editor.state.tr.setSelection(Selection.atEnd(editor.state.doc)))
measure()

/** The focus waits for tiptap to move the editor's page into place, which happens after mounting. */
let focusing: ReturnType<typeof setTimeout> | undefined
onMounted(() => {
  // On the page at last: how wide the text is can be measured.
  measure()
  if (props.autofocus) {
    focusing = setTimeout(() => autofocus({ focus: () => editor.commands.focus('end') }))
  }
})
onBeforeUnmount(() => {
  clearTimeout(focusing)
  editor.destroy()
})

watch(editable, (value) => editor.setEditable(value, false))
// A wider or narrower composer holds more or less of the message on its line.
watch(() => props.lineWidth, measure)
// Code's colors arrive with the highlighter and follow the page's mode.
watch(useMarkdownRenderVersion(), () => redrawDecorations(editor))
// A message from outside, such as a cleared draft: its files come with it.
watch(() => props.markdown, () => {
  if (props.markdown !== shown) {
    show()
  }
})

function holdsLines(doc: ProseMirrorNode): boolean {
  let lines = doc.childCount > 1
  doc.descendants((node) => {
    lines ||= node.type.spec.code === true || node.type.name === 'hardBreak' || node.type.name === 'image'
    return !lines
  })
  return lines
}

/** A line's last pixel is its own: rounding must not take a message off it. */
const LINE_SLACK_PX = 1

/** Tells the owner whether the message still fits one line of the composer. */
function measure(): void {
  if (holdsLines(editor.state.doc)) {
    multiline.value = true
    return
  }
  // A sent message has no line to fit, and a composer's is unknown until it is laid out.
  const line = props.lineWidth ?? 0
  multiline.value = line > 0 && textWidth() > line + LINE_SLACK_PX
}

/**
 * How wide the message is written out on one line. The editor's box is laid
 * out unwrapped to be measured and put back in the same frame, so nothing of
 * it is drawn; its scroll offsets are kept, since the shorter box drops them.
 */
function textWidth(): number {
  const dom = editor.view.dom as HTMLElement
  const { scrollLeft, scrollTop } = dom
  dom.style.width = 'max-content'
  dom.style.whiteSpace = 'pre'
  const width = dom.scrollWidth
  dom.style.width = ''
  dom.style.whiteSpace = ''
  dom.scrollLeft = scrollLeft
  dom.scrollTop = scrollTop
  return width
}

/** Tells the owner what the message now is. */
function write(): void {
  const { markdown, attachments } = serializeUserMarkdown<MessageCapsule>(editor.getJSON())
  shown = markdown
  measure()
  emit('change', markdown, attachments)
}

/** Shows a message that came from outside, such as a cleared draft; the undo history forgets what was there. */
function show(): void {
  const doc = editor.schema.nodeFromJSON(parseUserMarkdown(props.markdown, props.attachments))
  const tr = editor.state.tr.replaceWith(0, editor.state.doc.content.size, doc.content)
  tr.setSelection(Selection.atEnd(tr.doc))
  tr.setMeta('addToHistory', false)
  tr.setMeta('preventUpdate', true)
  editor.view.dispatch(tr)
  landing = null
  shown = serializeUserMarkdown<MessageCapsule>(editor.getJSON()).markdown
  measure()
}

function inCode(view: EditorView): boolean {
  return !!view.state.selection.$from.parent.type.spec.code
}

/** Pasted text read as the dialect: a single line joins the line at the cursor, code blocks stand on their own. */
function markdownSlice(schema: Schema, text: string): Slice {
  const doc = schema.nodeFromJSON(parseUserMarkdown(text, []))
  const opens = (node: ProseMirrorNode | null) => node?.type.name === 'paragraph' ? 1 : 0
  return new Slice(doc.content, opens(doc.firstChild), opens(doc.lastChild))
}

/** Where files land: here, with the selection they replace gone. */
function landAtSelection(view: EditorView): void {
  if (!view.state.selection.empty) {
    view.dispatch(view.state.tr.deleteSelection())
  }
  landing = view.state.selection.from
}

function paste(view: EditorView, event: ClipboardEvent): boolean {
  const transfer = event.clipboardData
  if (!transfer) {
    return false
  }
  const pasted = dataTransferFiles(transfer)
  if (pasted.length) {
    landAtSelection(view)
    emit('files', pasted)
    return true
  }
  // Code takes pasted text as it is.
  if (inCode(view)) {
    return false
  }
  const text = transfer.getData('text/plain')
  if (pastedTextIsLong(text)) {
    landAtSelection(view)
    emit('files', [pastedTextFile(text, capsuleNames())])
    return true
  }
  // Copied from a message: its own formatting and capsules.
  if (transfer.getData('text/html').includes('data-pm-slice')) {
    return false
  }
  view.dispatch(view.state.tr.replaceSelection(markdownSlice(view.state.schema, text)).scrollIntoView())
  return true
}

function drop(view: EditorView, event: DragEvent, moved: boolean): boolean {
  const transfer = event.dataTransfer
  // Files are the composer's to take, where they were dropped.
  if (transferHasFiles(transfer)) {
    return true
  }
  // A move inside the editor, or text dragged out of a message, carries its own nodes.
  if (moved || !transfer || transfer.getData('text/html').includes('data-pm-slice')) {
    return false
  }
  const text = transfer.getData('text/plain')
  const pos = view.posAtCoords({ left: event.clientX, top: event.clientY })?.pos
  if (!text || pos === undefined) {
    return false
  }
  const tr = view.state.tr.setSelection(TextSelection.create(view.state.doc, pos))
  view.dispatch(tr.replaceSelection(markdownSlice(view.state.schema, text)))
  return true
}

/**
 * Pasted or dropped nodes keep the capsules of files this composer carries
 * and does not have already: a capsule cut from this message can be pasted
 * back, one from elsewhere has no file here to send.
 */
function keepCapsules(fragment: Fragment, view: EditorView): Fragment {
  const present = new Set<string>()
  view.state.doc.descendants((node) => {
    const id = capsuleOf(node)?.id
    if (id) {
      present.add(id)
    }
  })
  const keep = (content: Fragment): Fragment => {
    const nodes: ProseMirrorNode[] = []
    content.forEach((node) => {
      const capsule = capsuleOf(node)
      if (!capsule) {
        nodes.push(node.isLeaf ? node : node.copy(keep(node.content)))
        return
      }
      if (transfers?.carries(capsule.id) && !present.has(capsule.id)) {
        present.add(capsule.id)
        nodes.push(node)
      }
    })
    return Fragment.from(nodes)
  }
  return keep(fragment)
}

/** The file a node stands for, when it is a capsule. */
function capsuleOf(node: ProseMirrorNode): MessageCapsule | null {
  return node.type.name === 'attachment' ? node.attrs['capsule'] ?? null : null
}

/** Copied text: each capsule as its file's name, each image as its alt text. */
function plainText(node: ProseMirrorNode): string {
  if (node.isText) {
    return node.text ?? ''
  }
  switch (node.type.name) {
    case 'hardBreak':
      return '\n'
    case 'attachment':
      return capsuleOf(node)?.name ?? ''
    case 'image':
      return String(node.attrs['alt'] ?? '')
  }
  const parts: string[] = []
  node.forEach((child) => parts.push(plainText(child)))
  return parts.join(node.isTextblock ? '' : '\n')
}

/**
 * A link opens as `file-previews.md` § Files named in messages says: in the
 * conversation on a click, in the composer on ⌘/Ctrl+click, since a click
 * there places the cursor.
 */
function click(event: MouseEvent): void {
  const link = event.target instanceof Element ? event.target.closest('a[data-link]') : null
  if (!link) {
    return
  }
  const kind = link.getAttribute('data-link')
  const target = link.getAttribute('data-target') ?? ''
  if (kind === 'web' && !editable.value) {
    return
  }
  event.preventDefault()
  if (editable.value && !event.metaKey && !event.ctrlKey) {
    return
  }
  if (kind === 'web') {
    window.open(target, '_blank', 'noopener,noreferrer')
    return
  }
  const current = files()
  const path = kind === 'file' && current ? messageHostPath(target, current.cwd) : null
  if (current && path !== null) {
    current.open(path)
  }
}

/** The names of the files in the message, which a new one must not take again. */
function capsuleNames(): string[] {
  const names: string[] = []
  editor.state.doc.descendants((node) => {
    const capsule = capsuleOf(node)
    if (capsule) {
      names.push(capsule.name)
    }
  })
  return names
}

defineExpose({
  /** Files are on their way: their capsules land where `event` dropped them, or at the cursor. */
  placeNextFiles(event?: DragEvent): void {
    if (!event) {
      landing = editor.state.selection.to
      return
    }
    // Dropped beside the text, they land at its end.
    landing = editor.view.posAtCoords({ left: event.clientX, top: event.clientY })?.pos
      ?? Selection.atEnd(editor.state.doc).from
  },
  /** Puts files into the message where they were told to land: one change, capsules and all. */
  insertCapsules(capsules: readonly MessageCapsule[]): void {
    if (!capsules.length) {
      return
    }
    const tr = editor.state.tr
    const $at = tr.doc.resolve(landing === null ? tr.selection.to : Math.min(landing, tr.doc.content.size))
    // A code block holds only text: a file dropped into one lands after it.
    const at = $at.parent.type.spec.code ? $at.after() : $at.pos
    tr.insert(at, capsules.map((capsule) => editor.schema.nodes['attachment']!.create({ capsule })))
    landing = null
    editor.view.dispatch(tr)
  },
  /** The names the message's files already have. */
  capsuleNames,
  focus(): void {
    editor.commands.focus()
  },
})
</script>

<template>
  <div
    class="message-editor"
    :data-files="files() ? '' : undefined"
    @click="click"
  >
    <EditorContent :editor="editor" />
  </div>
</template>
