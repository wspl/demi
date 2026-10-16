<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, watch } from 'vue'
import { DOMSerializer, Fragment, type Node as ProseMirrorNode, type Schema } from '@tiptap/pm/model'
import { Selection, type Transaction } from '@tiptap/pm/state'
import type { EditorView } from '@tiptap/pm/view'
import { Editor, EditorContent } from '@tiptap/vue-3'
import { messageFileTarget } from '../../markdown/filePath'
import { useMarkdownRenderVersion } from '../../markdown/highlight'
import { useMessageFiles } from '../../markdown/message-files'
import { ATTACHMENT_MARK, parseUserMarkdown, serializeUserMarkdown } from '../../markdown/user-markdown'
import { useAutofocus } from '../../ui/autofocus'
import { reportError } from '../../infra/errors'
import { t } from '../locales/text'
import { useMediaAddress } from '../media-source'
import {
  copiedMessageHtml,
  messageClipboard,
  type CopiedMessage,
  type MessageClipboard,
} from '../message-input/copied-message'
import { capsuleOf, useTransfers, type MessageCapsule } from './capsules'
import { composerExtensions, readOnlyExtensions, redrawDecorations } from './extensions'
import { fragmentDocument, markdownSlice, trimmedSlice } from './clipboard-slices'
import type { PlaceholderText } from '../../ui/ui-text'

/**
 * A user message, written or shown in one editor (`composer.md` § Writing a
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
    /**
     * What Up Arrow does while the composer holds no text and no files, read
     * at each press, such as opening the editor on the last message; absent,
     * the key moves the caret as usual.
     */
    editLast?: () => void
    placeholder?: PlaceholderText
    /** What assistive technology calls the field. */
    label?: string
    /**
     * How much width one line of the composer offers the text (`ComposerShell`
     * measures it): a message wider than that needs more than one line.
     */
    lineWidth?: number
    /** Takes the focus once it shows, the cursor at the end. */
    autofocus?: boolean
    /**
     * Counts the messages shown from outside: when it changes, the editor
     * shows `markdown` and `attachments` anew, even when the text is the one
     * it holds, since the same marks can stand for other files.
     */
    shownFromOutside?: number
  }>(),
  {
    attachments: () => [],
    placeholder: '',
  },
)

const emit = defineEmits<{
  /**
   * The message changed, typed or shown from outside: its Markdown, and the
   * files of its capsules in the order of their marks.
   */
  change: [markdown: string, attachments: MessageCapsule[]]
  /** Enter, or ⌘/Ctrl+Enter; `otherWay` for ⌘/Ctrl+Enter outside a code block (`MessageKeyOptions`). */
  submit: [otherWay: boolean]
  cancel: []
  /**
   * Files pasted, a paste long enough to be one, or images a paste or a drop
   * brought: the owner takes them and gives their capsules to
   * `insertCapsules`, at once, which puts them where they were pasted or
   * dropped.
   */
  files: [files: File[]]
  /**
   * A copied message pasted or dropped, in Demi's own form, with the picture
   * a copy of a lone image carried: the owner takes its files and gives the
   * message back to `insertMessage`, where it was pasted or dropped.
   */
  message: [message: CopiedMessage, image: File | null]
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
const mediaAddress = useMediaAddress()

/** The Markdown the editor last showed or wrote, so the same text coming back as a prop is not shown again. */
let shown = props.markdown
/** Where the next new capsules land: where files were dropped, or where the cursor was. */
let landing: number | null = null

const editor = new Editor({
  extensions: props.composer
    ? composerExtensions({
        placeholder: () => props.placeholder,
        submit: (otherWay) => emit('submit', otherWay),
        cancel: () => {
          if (props.cancelable) {
            emit('cancel')
          }
          return !!props.cancelable
        },
        editLast: () => {
          props.editLast?.()
          return !!props.editLast
        },
        intake: {
          carries: (id) => transfers?.carries(id) ?? false,
          land: (at) => {
            landing = at
          },
          files: (taken) => emit('files', taken),
          message: (message, picture) => emit('message', message, picture),
          content: insertContent,
        },
      })
    : readOnlyExtensions(),
  content: parseUserMarkdown(props.markdown, props.attachments),
  editable: editable.value,
  // Formatting as it is typed and pasted text follow the dialect, not tiptap's own rules.
  enableInputRules: false,
  enablePasteRules: false,
  editorProps: {
    // Read again on every update, so a composer that stops taking typing says so.
    attributes: () => ({
      class: 'markdown-body message-editor-text',
      ...(props.composer ? fieldAttributes() : {}),
    }),
    // A sent message copies with its files (`composer.md` § Writing a message);
    // the composer's own copy keeps ProseMirror's, which a paste in it reads.
    handleDOMEvents: props.composer ? {} : { copy: (view, event) => copySelection(view, event) },
    // A copy, a cut or a drag holds no blank line before its first line or
    // after its last, in its HTML as in its text; ProseMirror drags the
    // copied slice, so text moved within a composer leaves such lines behind.
    transformCopied: (slice, view) => trimmedSlice(slice, view.state.schema),
    // Copied text is the Markdown, each file by its name, wherever it is
    // copied from; code copied from inside a code block is the code.
    clipboardTextSerializer: (slice) => slice.openStart > 0 && slice.content.childCount === 1 && slice.content.firstChild?.type.spec.code
      ? slice.content.textBetween(0, slice.content.size)
      : messageClipboard(fragmentDocument(slice.content)).plain,
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
// The placeholder and the field's labels are read again on a state update, which a
// change of the page's language is not: an empty one shows the new words at once.
watch(() => [props.placeholder, props.label], () => editor.view.dispatch(editor.state.tr))
// A composer that changes shape, as a paste of many lines opens it, keeps
// the caret in view: the text scrolled while the composer was one line high,
// which leaves its box at the top once it holds six.
watch(multiline, () => {
  if (props.composer && editor.isFocused) {
    editor.view.dispatch(editor.state.tr.scrollIntoView())
  }
}, { flush: 'post' })
// A wider or narrower composer holds more or less of the message on its line.
watch(() => props.lineWidth, measure)
// Code's colors arrive with the highlighter and follow the page's mode.
watch(useMarkdownRenderVersion(), () => redrawDecorations(editor))
// A message from outside, such as a cleared draft or another page's: its
// files come with it.
watch(
  () => [props.markdown, props.shownFromOutside] as const,
  ([markdown, count], [, previous]) => {
    if (markdown !== shown || count !== previous) {
      show()
    }
  },
)

/**
 * The composer is a text field to assistive technology, as a textarea is
 * (WAI-ARIA's textbox role): an editable element has no role of its own. A
 * sent message is content, not a field.
 */
function fieldAttributes(): Record<string, string> {
  const attributes: Record<string, string> = { 'role': 'textbox', 'aria-multiline': 'true' }
  if (props.label) {
    attributes['aria-label'] = props.label
  }
  // The placeholder is drawn by CSS, which assistive technology does not read.
  if (props.placeholder) {
    attributes['aria-placeholder'] = props.placeholder
  }
  if (props.disabled) {
    attributes['aria-disabled'] = 'true'
  }
  // The product checks write in the composer by it, in any language (`browse.md`).
  if (props.composer) {
    attributes['data-composer'] = ''
  }
  return attributes
}

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

/**
 * Shows a message that came from outside, such as a cleared draft; the undo
 * history forgets what was there. The owner hears of it as of a typed one:
 * what it keeps of the message, such as the files a sent message took with
 * it, follows the document.
 */
function show(): void {
  const doc = editor.schema.nodeFromJSON(parseUserMarkdown(props.markdown, props.attachments))
  const tr = editor.state.tr.replaceWith(0, editor.state.doc.content.size, doc.content)
  tr.setSelection(Selection.atEnd(tr.doc))
  tr.setMeta('addToHistory', false)
  tr.setMeta('preventUpdate', true)
  editor.view.dispatch(tr)
  landing = null
  write()
}

function inCode(view: EditorView): boolean {
  return !!view.state.selection.$from.parent.type.spec.code
}

/**
 * A copy's forms: Demi's own and plain text, and its HTML, Demi's own form an
 * attribute of its outermost element; all three hold the same lines, without
 * blank ones at the ends.
 */
function clipboardForms(content: Fragment): { clipboard: MessageClipboard; html: string } {
  const clipboard = messageClipboard(fragmentDocument(content))
  const html = htmlSerializer(editor.schema).serializeFragment(Fragment.fromJSON(editor.schema, clipboard.doc.content ?? []))
  return { clipboard, html: copiedMessageHtml(clipboard.message, html) }
}

/** The message as HTML for mail, documents and chat apps: its formatting, each picture an image, every other file its name. */
function htmlSerializer(schema: Schema): DOMSerializer {
  const base = DOMSerializer.fromSchema(schema)
  return new DOMSerializer({
    ...base.nodes,
    attachment: (node) => {
      const capsule = capsuleOf(node)
      const src = capsule ? pictureAddress(capsule) : null
      const name = capsule?.name ?? ''
      return src ? ['img', { src, alt: name }] : ['span', name]
    },
  }, base.marks)
}

/** Where a capsule's picture loads from, as an address any page can follow; null for a file without one. */
function pictureAddress(capsule: MessageCapsule): string | null {
  const image = capsule.image
  if (!image) {
    return null
  }
  return new URL(typeof image === 'string' ? image : mediaAddress(image), document.baseURI).href
}

/**
 * The clipboard item of a copy: its forms, and the picture itself when it
 * holds a lone image, so pasting it anywhere pastes the picture.
 */
function clipboardItem(forms: { clipboard: MessageClipboard; html: string }): ClipboardItem {
  const image = forms.clipboard.image
  const src = image ? pictureAddress(image) : null
  return new ClipboardItem({
    'text/plain': new Blob([forms.clipboard.plain], { type: 'text/plain' }),
    'text/html': new Blob([forms.html], { type: 'text/html' }),
    ...(src ? { 'image/png': pngOf(src) } : {}),
  })
}

/** A picture as PNG, the one image type every web browser's clipboard takes. */
async function pngOf(src: string): Promise<Blob> {
  const response = await fetch(src, { credentials: 'include' })
  if (!response.ok) {
    throw new Error(`The picture did not load (${response.status}).`)
  }
  const blob = await response.blob()
  if (blob.type === 'image/png') {
    return blob
  }
  const bitmap = await createImageBitmap(blob)
  try {
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height)
    canvas.getContext('2d')?.drawImage(bitmap, 0, 0)
    return await canvas.convertToBlob({ type: 'image/png' })
  } finally {
    bitmap.close()
  }
}

/** Whether the page's selection runs past this message, into the answer or another message. */
function selectionLeaves(view: EditorView): boolean {
  const selection = view.dom.ownerDocument.getSelection()
  if (!selection || selection.rangeCount === 0) {
    return false
  }
  const range = selection.getRangeAt(0)
  return !view.dom.contains(range.startContainer) || !view.dom.contains(range.endContainer)
}

/** A selection in a sent message copies in every form; a lone picture also as itself. */
function copySelection(view: EditorView, event: ClipboardEvent): boolean {
  // A selection that runs on past the message is the page's to copy, whole and
  // in order, as any web page copies it: ProseMirror's own copy, which a
  // handler that declines would get, holds only the part in this message.
  if (selectionLeaves(view)) {
    return true
  }
  const transfer = event.clipboardData
  const { selection } = view.state
  // Code copied from inside a code block is the code, as ProseMirror copies it.
  if (selection.empty || !transfer || (selection.$from.sameParent(selection.$to) && inCode(view))) {
    return false
  }
  const forms = clipboardForms(selection.content().content)
  event.preventDefault()
  transfer.setData('text/plain', forms.clipboard.plain)
  transfer.setData('text/html', forms.html)
  // A picture cannot go into the copy event's data; the asynchronous
  // clipboard takes it, with the forms above, while the copy is a gesture.
  if (forms.clipboard.image) {
    void navigator.clipboard.write([clipboardItem(forms)])
      .catch((error: unknown) => reportError(t('attachments.couldntCopyPicture'), error))
  }
  return true
}

/**
 * Where new content lands: where it was told to, or at the cursor; a code
 * block holds only text, so content that lands in one goes after it.
 */
function landingPlace(tr: Transaction): number {
  const $at = tr.doc.resolve(landing === null ? tr.selection.from : Math.min(landing, tr.doc.content.size))
  return $at.parent.type.spec.code ? $at.after() : $at.pos
}

/** Puts files into the message where they were told to land: one change, capsules and all. */
function insertCapsules(capsules: readonly MessageCapsule[]): void {
  if (!capsules.length || editor.isDestroyed) {
    return
  }
  const tr = editor.state.tr
  const at = landingPlace(tr)
  tr.insert(at, capsules.map((capsule) => editor.schema.nodes['attachment']!.create({ capsule })))
  landing = null
  editor.view.dispatch(tr)
}

/**
 * A pasted message, its files taken: its text and capsules go in where the
 * paste landed, as one change, the cursor after them.
 */
function insertMessage(markdown: string, capsules: readonly MessageCapsule[]): void {
  if (editor.isDestroyed) {
    return
  }
  const tr = editor.state.tr
  const at = landingPlace(tr)
  tr.replaceRange(at, at, markdownSlice(editor.schema, markdown, capsules))
  tr.setSelection(Selection.near(tr.doc.resolve(tr.mapping.map(at, 1)), -1))
  landing = null
  editor.view.dispatch(tr.scrollIntoView())
}

/**
 * Text with the images a paste or a drop brought, each where it stood: the
 * text goes in, and each image is taken as a pasted file is, its capsule
 * landing after the text before it. The owner gives the capsules back at
 * once, so the steps follow each other as one change to undo.
 */
function insertContent(markdown: string, taken: File[]): void {
  let at: number | null = landing
  markdown.split(ATTACHMENT_MARK).forEach((part, index) => {
    const file = index > 0 ? taken[index - 1] : undefined
    if (file) {
      landing = at
      const size = editor.state.doc.content.size
      emit('files', [file])
      at = at === null ? null : at + editor.state.doc.content.size - size
    }
    if (part) {
      landing = at
      insertMessage(part, [])
      at = editor.state.selection.to
    }
  })
  landing = null
}

/**
 * A drag out of a message carries Demi's own form beside ProseMirror's, as
 * a copy does, so another composer brings its files back as capsules
 * without uploading them; this composer moves what it dragged by
 * ProseMirror's own record of the drag.
 */
function dragStart(event: DragEvent): void {
  const slice = editor.view.dragging?.slice
  const transfer = event.dataTransfer
  if (!slice || !transfer) {
    return
  }
  const { clipboard } = clipboardForms(slice.content)
  if (!clipboard.message.files.length) {
    return
  }
  const html = document.createRange().createContextualFragment(transfer.getData('text/html'))
  transfer.setData('text/html', copiedMessageHtml(clipboard.message, html))
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
  const file = kind === 'file' && current ? messageFileTarget(target, current) : null
  if (current?.open && file !== null) {
    current.open(file)
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
  insertCapsules,
  insertMessage,
  /**
   * A pasted file the composer could not add: its name stands where its
   * capsule did. Answers whether the message still held that capsule.
   */
  replaceCapsule(id: string, text: string): boolean {
    if (editor.isDestroyed) {
      return false
    }
    let at: number | null = null
    editor.state.doc.descendants((node, pos) => {
      if (at === null && capsuleOf(node)?.id === id) {
        at = pos
      }
      return at === null
    })
    if (at === null) {
      return false
    }
    editor.view.dispatch(editor.state.tr.replaceWith(at, at + 1, editor.schema.text(text)))
    return true
  },
  /** The whole message as the clipboard takes it, for its Copy control. */
  clipboardItem(): ClipboardItem {
    return clipboardItem(clipboardForms(editor.state.doc.content))
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
    @dragstart="dragStart"
  >
    <EditorContent :editor="editor" />
  </div>
</template>
