<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, watch } from 'vue'
import { Fragment, type Node as ProseMirrorNode, type Slice } from '@tiptap/pm/model'
import { type EditorState, Selection, type Transaction } from '@tiptap/pm/state'
import { Editor, EditorContent } from '@tiptap/vue-3'
import { useMarkdownRenderVersion } from '../../markdown/render-version'
import { useMessageFiles } from '../../markdown/message-files'
import { ATTACHMENT_MARK, parseTypedMarkdown, serializeTypedMarkdown } from '../../markdown/user-markdown'
import { useAutofocus } from '../../ui/autofocus'
import { copiedMessageHtml, messageClipboard, type CopiedMessage } from '../message-input/copied-message'
import { capsuleOf, useTransfers, type MessageCapsule } from './capsules'
import { composerExtensions, inCode, redrawDecorations } from './extensions'
import { fragmentDocument, markdownSlice } from './clipboard-slices'
import { useMessageClipboard } from './message-clipboard'
import { followMessageLink } from './message-links'
import type { PlaceholderText } from '../../ui/ui-text'

/**
 * The composer's editor, where a user message is written: its Markdown as
 * typed, every character kept, with the dialect's styles drawn over it, and
 * each file a capsule where it stands. A sent message is drawn from the
 * dialect's own reading of it (`SentMessage`).
 */
const props = withDefaults(
  defineProps<{
    /** The message's Markdown, an attachment mark where each capsule stands. */
    markdown: string
    /**
     * Its files, in the order of their marks; the document keeps them from
     * there on.
     */
    attachments?: readonly MessageCapsule[]
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
    /**
     * Takes the focus once it shows, the cursor at the end; on a phone only
     * when `asked`.
     */
    autofocus?: boolean
    /**
     * Typing here is what the user just chose, as Edit on a message is: it
     * takes the focus on a phone too.
     */
    asked?: boolean
    /**
     * Counts the messages shown from outside: when it changes, the editor
     * shows `markdown` and `attachments` anew, even when the text is the one
     * it holds, since the same marks can stand for other files.
     */
    shownFromOutside?: number
    /**
     * The document the editor is made in, a frame's (`FramedField.vue`);
     * the page's when absent. ProseMirror listens to the document its editor
     * is made in.
     */
    document?: Document
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
  /**
   * Enter, or ⌘/Ctrl+Enter; `otherWay` for ⌘/Ctrl+Enter outside a code block
   * (`MessageKeyOptions`).
   */
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
const autofocus = useAutofocus({ asked: () => props.asked })
const editable = computed(() => !props.disabled)
const transfers = useTransfers()
const clipboard = useMessageClipboard()

/**
 * The Markdown the editor last showed or wrote, so the same text coming back as
 * a prop is not shown again.
 */
let shown = props.markdown
/**
 * Where the next new capsules land: where files were dropped, or where the
 * cursor was.
 */
let landing: number | null = null
/** The slice ProseMirror last put on the clipboard, until `copy` takes it. */
let copied: Slice | null = null

const editor = new Editor({
  ...(props.document ? { element: props.document.createElement('div') } : {}),
  extensions: composerExtensions({
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
  }),
  content: parseTypedMarkdown(props.markdown, props.attachments),
  editable: editable.value,
  // What is typed and pasted stays as it is: tiptap's rules would turn it
  // into something else.
  enableInputRules: false,
  enablePasteRules: false,
  editorProps: {
    // Read again on every update, so a composer that stops taking typing says
    // so.
    attributes: (state) => ({
      class: 'markdown-body message-editor-text',
      ...fieldAttributes(state),
    }),
    // A copy, a cut or a drag holds what was selected, as a text field's
    // does.
    transformCopied: (slice) => {
      copied = slice
      return slice
    },
    // Copied text is the text as typed, each file by its name.
    clipboardTextSerializer: (slice) => messageClipboard(fragmentDocument(slice.content)).plain,
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

/** Takes the focus as the page gives it, the cursor at the end. */
function takeFocus(): void {
  autofocus({ element: editor.view.dom, focus: () => editor.commands.focus('end') })
}

/**
 * The focus waits for tiptap to move the editor's page into place, which
 * happens after mounting.
 */
let focusing: ReturnType<typeof setTimeout> | undefined
/**
 * The editor's box as it is laid out, which a frame's styles arriving, its
 * fonts loading or its owner's shape changing moves after it mounted: the
 * text is measured again whenever it does.
 */
let resized: ResizeObserver | undefined
onMounted(() => {
  // On the page at last: how wide the text is can be measured.
  measure()
  const view = editor.view.dom.ownerDocument.defaultView ?? window
  resized = new view.ResizeObserver(() => measure())
  resized.observe(editor.view.dom)
  if (props.autofocus) {
    focusing = setTimeout(() => takeFocus())
  }
})
onBeforeUnmount(() => {
  resized?.disconnect()
  clearTimeout(focusing)
  editor.destroy()
})

watch(editable, (value) => editor.setEditable(value, false))
// The placeholder and the field's labels are read again on a state update,
// which a change of the page's language is not: an empty one shows the new
// words at once.
watch(() => [props.placeholder, props.label], () => editor.view.dispatch(editor.state.tr))
// A composer that changes shape, as a paste of many lines opens it, keeps
// the caret in view: the text scrolled while the composer was one line high,
// which leaves its box at the top once it holds six.
watch(multiline, () => {
  if (editor.isFocused) {
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
 * (WAI-ARIA's textbox role): an editable element has no role of its own.
 */
function fieldAttributes(state: EditorState): Record<string, string> {
  const attributes: Record<string, string> = {
    'role': 'textbox',
    'aria-multiline': 'true',
    // A touch keyboard's return key says what Enter does here: it sends,
    // and in code between fences it starts a line.
    'enterkeyhint': inCode(state) ? 'enter' : 'send',
  }
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
  // The product checks write in the composer by it, in any language
  // (`browse.md`).
  attributes['data-composer'] = ''
  return attributes
}

function holdsLines(doc: ProseMirrorNode): boolean {
  return doc.childCount > 1
}

/** A line's last pixel is its own: rounding must not take a message off it. */
const LINE_SLACK_PX = 1

/** Tells the owner whether the message still fits one line of the composer. */
function measure(): void {
  if (holdsLines(editor.state.doc)) {
    multiline.value = true
    return
  }
  // The composer's line is unknown until it is laid out.
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
  const { markdown, attachments } = serializeTypedMarkdown<MessageCapsule>(editor.getJSON())
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
  const doc = editor.schema.nodeFromJSON(parseTypedMarkdown(props.markdown, props.attachments))
  const tr = editor.state.tr.replaceWith(0, editor.state.doc.content.size, doc.content)
  tr.setSelection(Selection.atEnd(tr.doc))
  tr.setMeta('addToHistory', false)
  tr.setMeta('preventUpdate', true)
  editor.view.dispatch(tr)
  landing = null
  write()
}

/** Where new content lands: where it was told to, or at the cursor. */
function landingPlace(tr: Transaction): number {
  return landing === null ? tr.selection.from : Math.min(landing, tr.doc.content.size)
}

/**
 * Puts files into the message where they were told to land: one change,
 * capsules and all.
 */
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
  const forms = clipboard.forms(editor.schema, currentFiles(slice.content))
  if (!forms.clipboard.message.files.length) {
    return
  }
  const html = document.createRange().createContextualFragment(transfer.getData('text/html'))
  transfer.setData('text/html', copiedMessageHtml(forms.clipboard.message, html))
}

/**
 * `content` with each capsule as the composer knows its file now: a file
 * uploaded since its capsule went in has its blob, which a copy carries.
 */
function currentFiles(content: Fragment): Fragment {
  const nodes: ProseMirrorNode[] = []
  content.forEach((node) => {
    const capsule = capsuleOf(node)
    const known = capsule && transfers?.current(capsule.id)
    nodes.push(known
      ? node.type.create({ ...node.attrs, capsule: known }, node.content, node.marks)
      : node.isLeaf ? node : node.copy(currentFiles(node.content)))
  })
  return Fragment.from(nodes)
}

/**
 * A copy or a cut carries Demi's own form beside ProseMirror's, as a drag
 * does, so another conversation's composer brings its files back as capsules
 * without uploading them. It runs once ProseMirror has written the clipboard,
 * from the slice it copied, which a cut has already taken from the message.
 */
function copy(event: ClipboardEvent): void {
  const slice = copied
  copied = null
  const transfer = event.clipboardData
  if (!slice || !transfer) {
    return
  }
  const forms = clipboard.forms(editor.schema, currentFiles(slice.content))
  if (!forms.clipboard.message.files.length) {
    return
  }
  const html = document.createRange().createContextualFragment(transfer.getData('text/html'))
  transfer.setData('text/html', copiedMessageHtml(forms.clipboard.message, html))
}

/**
 * A link opens as `followMessageLink` says, in the composer on ⌘/Ctrl+click.
 */
function click(event: MouseEvent): void {
  followMessageLink(event, editable.value, files())
}

defineExpose({
  /**
   * Files are on their way: their capsules land where `event` dropped them, or
   * at the cursor.
   */
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
  focus(): void {
    editor.commands.focus()
  },
  /**
   * Adds `text` as plain text at the end, as a line of its own after what is
   * there, which then changes the message as typing does.
   */
  appendLine(text: string): void {
    const empty = editor.state.doc.textContent.length === 0 && editor.state.doc.childCount <= 1
    const chain = editor.chain().focus('end')
    if (!empty) {
      chain.splitBlock()
    }
    chain.insertContent({ type: 'text', text }).run()
  },
  /**
   * Takes the focus as the page gives it, the cursor at the end: not on a phone
   * unless `asked` (`useAutofocus`).
   */
  takeFocus,
})
</script>

<template>
  <div
    class="message-editor"
    :data-files="files() ? '' : undefined"
    @click="click"
    @dragstart="dragStart"
    @copy="copy"
    @cut="copy"
  >
    <EditorContent :editor="editor" />
  </div>
</template>
