<script setup lang="ts">
import { computed, h, ref, type VNode, type VNodeChild } from 'vue'
import type { Node as ProseMirrorNode, Slice } from '@tiptap/pm/model'
import { useMarkdownRenderVersion } from '../../markdown/render-version'
import { useMessageFiles } from '../../markdown/message-files'
import { parseTypedMarkdown } from '../../markdown/user-markdown'
import { reportError } from '../../infra/errors'
import { t } from '../locales/text'
import type { MessageCapsule } from './capsules'
import { messageSchema } from './extensions'
import { messageHighlights, type HighlightAttributes, type LineHighlight } from './highlights'
import MessageAttachment from './MessageAttachment.vue'
import { useMessageClipboard } from './message-clipboard'
import { followMessageLink } from './message-links'

/**
 * A sent message as the conversation shows it: its lines as typed, with the
 * highlighting its editor draws (`highlights.ts`), each file as its capsule,
 * and no editor behind it.
 *
 * Its text selects as any text does. A copy or a drag of it, or of a capsule,
 * carries that part of the message in every form, its files with it, as the
 * composer's own copy does. Each line and file it draws says where it stands
 * in the message's document, so the page's selection maps to the document's
 * positions, and the document's slice between them is what goes, as
 * ProseMirror's own copy takes it.
 */
const props = withDefaults(defineProps<{
  /** The message's Markdown, an attachment mark where each capsule stands. */
  markdown: string
  /** Its files, in the order of their marks. */
  attachments?: readonly MessageCapsule[]
}>(), {
  attachments: () => [],
})

const schema = messageSchema()
const files = useMessageFiles()
const clipboard = useMessageClipboard()
const renderVersion = useMarkdownRenderVersion()
const root = ref<HTMLElement>()

const doc = computed(() => schema.nodeFromJSON(parseTypedMarkdown(props.markdown, props.attachments)))
/**
 * Where a drawn node stands in the message's document: the position before it.
 */
const POSITION = 'data-pos'

/** An element drawn with highlighting's attributes, `nodeName` its tag, a span by default. */
function element(attributes: HighlightAttributes, children: VNodeChild[]): VNode {
  const { nodeName = 'span', ...rest } = attributes
  return h(nodeName, rest, children)
}

/**
 * The line's content from offset `from` to `to`, a character of text one and
 * a capsule one, as plain text and the files drawn where they stand.
 */
function drawPlain(line: ProseMirrorNode, start: number, from: number, to: number): VNodeChild[] {
  const parts: VNodeChild[] = []
  line.forEach((child, offset) => {
    const end = offset + child.nodeSize
    if (end <= from || offset >= to) {
      return
    }
    if (child.isText) {
      parts.push((child.text ?? '').slice(Math.max(0, from - offset), to - offset))
    } else {
      parts.push(drawFile(child, start + offset))
    }
  })
  return parts
}

/**
 * The line from `from` to `to` with the ranges inside it, each wrapping the
 * ranges it holds; the ranges come in order of their start, the longer of
 * two that start together first, as constructs nest.
 */
function drawRanges(
  line: ProseMirrorNode,
  start: number,
  from: number,
  to: number,
  ranges: LineHighlight['ranges'],
): VNodeChild[] {
  const parts: VNodeChild[] = []
  let at = from
  let index = 0
  while (index < ranges.length) {
    const range = ranges[index]!
    const end = Math.min(range.to, to)
    let inner = index + 1
    while (inner < ranges.length && ranges[inner]!.from < end) {
      inner += 1
    }
    parts.push(...drawPlain(line, start, at, range.from))
    parts.push(element(range.attributes, drawRanges(line, start, range.from, end, ranges.slice(index + 1, inner))))
    at = end
    index = inner
  }
  parts.push(...drawPlain(line, start, at, to))
  return parts.filter((part) => part !== '')
}

/** A capsule, which stands at `pos`, drawn. */
function drawFile(node: ProseMirrorNode, pos: number): VNodeChild {
  const at = { [POSITION]: String(pos) }
  return h('span', { ...at, 'data-attachment': '', draggable: 'true' }, [
    h(MessageAttachment, { saved: node.attrs['capsule'] ?? null, editable: false }),
  ])
}

/**
 * Each line of the message as a paragraph that says where it stands, with
 * its highlighting; an empty line holds a line's height, as the editor gives
 * it.
 */
function drawLines(message: ProseMirrorNode): VNodeChild[] {
  const highlights = messageHighlights(message)
  const lines: VNodeChild[] = []
  message.forEach((line, offset, index) => {
    const highlight = highlights[index] ?? { line: null, ranges: [] }
    const ranges = [...highlight.ranges].sort((a, b) => a.from - b.from || b.to - a.to)
    const content = line.content.size ? drawRanges(line, offset + 1, 0, line.content.size, ranges) : [h('br')]
    lines.push(h('p', { ...(highlight.line ?? {}), [POSITION]: String(offset) }, content))
  })
  return lines
}

/**
 * The message drawn, again as the highlighter's colors arrive or the page's
 * mode changes.
 */
function Content(): VNodeChild {
  void renderVersion.value
  return drawLines(doc.value)
}

/** Where a drawn element's node stands in the document. */
function positionAttribute(element: Element): number {
  return Number(element.getAttribute(POSITION))
}

/**
 * How far into its line a point lies, as the document counts it: each
 * character of text one, each node within the line (a capsule, a picture,
 * a line break) one however it draws, and what the page drew besides its
 * nodes, such as a mark's element or an empty line's break, nothing.
 */
function lineOffset(line: Element, container: Node, offset: number): number {
  const range = document.createRange()
  range.setStart(line, 0)
  range.setEnd(container, offset)
  let count = 0
  const visit = (node: Node): void => {
    if (node instanceof Text) {
      count += node.length
    } else if (node instanceof Element && node.hasAttribute(POSITION)) {
      count += 1
    } else {
      node.childNodes.forEach(visit)
    }
  }
  range.cloneContents().childNodes.forEach(visit)
  return count
}

/**
 * The document position of a point of the page's selection in the message,
 * as ProseMirror's own `posAtDOM` gives it: within a line, by its offset;
 * inside a capsule or a picture, before it when the point starts the
 * selection and after it when it ends it; among blocks, before the next one
 * or at the end of the one that holds them.
 */
function positionAt(container: Node, offset: number, side: 'start' | 'end'): number {
  const element = root.value!
  const holder = (container instanceof Element ? container : container.parentElement)?.closest(`[${POSITION}]`)
  if (holder && element.contains(holder)) {
    const pos = positionAttribute(holder)
    const node = doc.value.nodeAt(pos)!
    if (node.isTextblock) {
      return pos + 1 + lineOffset(holder, container, offset)
    }
    if (node.isInline) {
      return side === 'start' ? pos : pos + node.nodeSize
    }
  }
  const parent = holder && element.contains(holder) ? holder : element
  const children = container instanceof Element ? [...container.childNodes].slice(offset) : []
  for (const child of children) {
    const next = child instanceof Element ? (child.hasAttribute(POSITION) ? child : child.querySelector(`[${POSITION}]`)) : null
    if (next) {
      return positionAttribute(next)
    }
  }
  return parent === element ? doc.value.content.size : positionAttribute(parent) + doc.value.nodeAt(positionAttribute(parent))!.nodeSize - 1
}

/**
 * The part of the message `range` selects, as the document's slice between its
 * ends.
 */
function selected(range: Range): Slice | null {
  const from = positionAt(range.startContainer, range.startOffset, 'start')
  const to = positionAt(range.endContainer, range.endOffset, 'end')
  return to > from ? doc.value.slice(from, to) : null
}

/** The page's selection when it lies within this message. */
function ownSelection(): Range | null {
  const selection = document.getSelection()
  const element = root.value
  if (!selection || selection.rangeCount === 0 || selection.isCollapsed || !element) {
    return null
  }
  const range = selection.getRangeAt(0)
  // A selection that runs on past the message is the page's to copy, whole and
  // in order.
  if (!element.contains(range.startContainer) || !element.contains(range.endContainer)) {
    return null
  }
  return range
}

/**
 * A selection in a sent message copies in every form; a lone picture also as
 * itself.
 */
function copy(event: ClipboardEvent): void {
  const range = ownSelection()
  const transfer = event.clipboardData
  if (!range || !transfer) {
    return
  }
  const content = selected(range)
  if (!content) {
    return
  }
  const forms = clipboard.forms(schema, content.content)
  event.preventDefault()
  transfer.setData('text/plain', forms.clipboard.plain)
  transfer.setData('text/html', forms.html)
  // A picture cannot go into the copy event's data; the asynchronous
  // clipboard takes it, with the forms above, while the copy is a gesture.
  if (forms.clipboard.image) {
    void navigator.clipboard.write([clipboard.item(forms)])
      .catch((error: unknown) => reportError(t('attachments.couldntCopyPicture'), error, { site: 'attachments.couldntCopyPicture' }))
  }
}

/**
 * A drag of a capsule, or of the selected text, carries the message in
 * every form, its files with it, so a composer brings them back as capsules
 * without uploading them.
 */
function dragStart(event: DragEvent): void {
  const transfer = event.dataTransfer
  const host = event.target instanceof Element ? event.target.closest('[data-attachment]') : null
  let content: Slice | null = null
  if (host && root.value?.contains(host)) {
    const pos = positionAttribute(host)
    content = doc.value.slice(pos, pos + 1)
  } else {
    const range = ownSelection()
    content = range ? selected(range) : null
  }
  if (!transfer || !content) {
    return
  }
  const forms = clipboard.forms(schema, content.content)
  transfer.setData('text/plain', forms.clipboard.plain)
  transfer.setData('text/html', forms.html)
}

defineExpose({
  /** The whole message as the clipboard takes it, for its Copy control. */
  clipboardItem(): ClipboardItem {
    return clipboard.item(clipboard.forms(schema, doc.value.content))
  },
})
</script>

<template>
  <div
    class="message-editor"
    :data-files="files() ? '' : undefined"
  >
    <div
      ref="root"
      class="markdown-body message-editor-text sent-message"
      @click="followMessageLink($event, false, files())"
      @copy="copy"
      @dragstart="dragStart"
    >
      <Content />
    </div>
  </div>
</template>
