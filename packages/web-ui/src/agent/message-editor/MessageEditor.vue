<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, watch } from 'vue'
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
import { provideCapsules, type MessageCapsule } from './capsules'
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
    /** Its files, in the order of their marks. */
    capsules?: readonly MessageCapsule[]
    /** The composer's editor: formatting as it is typed, Enter to send, files dropped and pasted. */
    composer?: boolean
    /** A composer that takes no typing for now, such as an edit being sent. */
    disabled?: boolean
    /** Escape ends something, such as an edit. */
    cancelable?: boolean
    placeholder?: string
    /** What assistive technology calls the field. */
    label?: string
    /** Takes the focus once it shows, the cursor at the end. */
    autofocus?: boolean
  }>(),
  {
    capsules: () => [],
    placeholder: '',
  },
)

const emit = defineEmits<{
  /** The text or its capsules changed: the Markdown, and the capsule ids in the order of their marks. */
  change: [markdown: string, attachmentIds: string[]]
  submit: []
  cancel: []
  /** Files pasted, or a paste long enough to be one; their capsules land where the cursor was. */
  files: [files: File[]]
  retry: [id: string]
  focus: []
  blur: []
}>()

/** The message holds more than one line of text: a line break, a code block or an image. */
const multiline = defineModel<boolean>('multiline', { default: false })

const files = useMessageFiles()
const autofocus = useAutofocus()
const editable = computed(() => !!props.composer && !props.disabled)
const capsuleById = computed(() => new Map(props.capsules.map((capsule) => [capsule.id, capsule])))
provideCapsules({
  capsule: (id) => capsuleById.value.get(id),
  editable: () => editable.value,
  retry: (id) => emit('retry', id),
})

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
  content: parseUserMarkdown(props.markdown, props.capsules.map((capsule) => capsule.id)),
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
multiline.value = holdsLines(editor.state.doc)

/** The focus waits for tiptap to move the editor's page into place, which happens after mounting. */
let focusing: ReturnType<typeof setTimeout> | undefined
onMounted(() => {
  if (props.autofocus) {
    focusing = setTimeout(() => autofocus({ focus: () => editor.commands.focus('end') }))
  }
})
onBeforeUnmount(() => {
  clearTimeout(focusing)
  editor.destroy()
})

watch(editable, (value) => editor.setEditable(value, false))
// Code's colors arrive with the highlighter and follow the page's mode.
watch(useMarkdownRenderVersion(), () => redrawDecorations(editor))
// The text and which files it holds, not the files' progress, which their capsules show on their own.
watch(
  () => `${props.markdown}\n${props.capsules.map((capsule) => capsule.id).join('\n')}`,
  () => {
    if (props.markdown === shown) {
      placeCapsules()
    } else {
      show()
    }
  },
)

function holdsLines(doc: ProseMirrorNode): boolean {
  let lines = doc.childCount > 1
  doc.descendants((node) => {
    lines ||= node.type.spec.code === true || node.type.name === 'hardBreak' || node.type.name === 'image'
    return !lines
  })
  return lines
}

/** Tells the owner what the message now is. */
function write(): void {
  // Undo can bring back a capsule whose file was removed with it: it goes again, and that change is written instead.
  const tr = editor.state.tr
  dropStrayCapsules(tr)
  if (tr.docChanged) {
    editor.view.dispatch(tr.setMeta('addToHistory', false))
    return
  }
  const { markdown, attachmentIds } = serializeUserMarkdown(editor.getJSON())
  shown = markdown
  multiline.value = holdsLines(editor.state.doc)
  emit('change', markdown, attachmentIds)
}

/** Shows Markdown that came from outside, such as a cleared draft or another message; the undo history forgets what was there. */
function show(): void {
  const ids = props.capsules.map((capsule) => capsule.id)
  const doc = editor.schema.nodeFromJSON(parseUserMarkdown(props.markdown, ids))
  const tr = editor.state.tr.replaceWith(0, editor.state.doc.content.size, doc.content)
  tr.setSelection(Selection.atEnd(tr.doc))
  tr.setMeta('addToHistory', false)
  tr.setMeta('preventUpdate', true)
  editor.view.dispatch(tr)
  landing = null
  const written = serializeUserMarkdown(editor.getJSON())
  shown = written.markdown
  multiline.value = holdsLines(editor.state.doc)
  // A mark without a file is dropped and a file without a mark lands at the end: the owner hears the result.
  if (written.markdown !== props.markdown || written.attachmentIds.join('\n') !== ids.join('\n')) {
    emit('change', written.markdown, written.attachmentIds)
  }
}

/** Deletes capsules whose file the owner no longer has, and a second capsule of one file; the ids that stay. */
function dropStrayCapsules(tr: Transaction): Set<string> {
  const present = new Set<string>()
  tr.doc.descendants((node, pos) => {
    if (node.type.name !== 'attachment') {
      return true
    }
    const id = String(node.attrs['id'])
    if (capsuleById.value.has(id) && !present.has(id)) {
      present.add(id)
    } else {
      tr.delete(tr.mapping.map(pos), tr.mapping.map(pos + node.nodeSize))
    }
    return false
  })
  return present
}

/**
 * Keeps the capsules to the owner's files: a capsule whose file is gone goes,
 * and a new file's capsule lands where files were dropped, or at the cursor.
 */
function placeCapsules(): void {
  const tr = editor.state.tr
  const present = dropStrayCapsules(tr)
  const added = props.capsules.filter((capsule) => !present.has(capsule.id))
  if (added.length) {
    const $at = tr.doc.resolve(landing === null ? tr.selection.to : tr.mapping.map(landing))
    // A code block holds only text: a file dropped into one lands after it.
    const at = $at.parent.type.spec.code ? $at.after() : $at.pos
    tr.insert(at, added.map((capsule) => editor.schema.nodes['attachment']!.create({ id: capsule.id })))
    landing = null
  }
  if (tr.docChanged) {
    editor.view.dispatch(tr.setMeta('addToHistory', false))
  }
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
    emit('files', [pastedTextFile(text, props.capsules.map((capsule) => capsule.name))])
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

/** Pasted or dropped nodes keep only capsules of this editor's files that are not in it already. */
function keepCapsules(fragment: Fragment, view: EditorView): Fragment {
  const present = new Set<string>()
  view.state.doc.descendants((node) => {
    if (node.type.name === 'attachment') {
      present.add(String(node.attrs['id']))
    }
  })
  const keep = (content: Fragment): Fragment => {
    const nodes: ProseMirrorNode[] = []
    content.forEach((node) => {
      if (node.type.name !== 'attachment') {
        nodes.push(node.isLeaf ? node : node.copy(keep(node.content)))
        return
      }
      const id = String(node.attrs['id'])
      if (capsuleById.value.has(id) && !present.has(id)) {
        present.add(id)
        nodes.push(node)
      }
    })
    return Fragment.from(nodes)
  }
  return keep(fragment)
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
      return capsuleById.value.get(String(node.attrs['id']))?.name ?? ''
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
