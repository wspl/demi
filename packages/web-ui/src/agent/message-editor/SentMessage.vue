<script setup lang="ts">
import { computed, h, ref, type VNode, type VNodeChild } from 'vue'
import type { DOMOutputSpec, Node as ProseMirrorNode, Slice } from '@tiptap/pm/model'
import { useMarkdownRenderVersion } from '../../markdown/render-version'
import { useMessageFiles } from '../../markdown/message-files'
import { parseUserMarkdown } from '../../markdown/user-markdown'
import { reportError } from '../../infra/errors'
import { t } from '../locales/text'
import type { MessageCapsule } from './capsules'
import { bareLinks, codeColors, messageSchema } from './extensions'
import { MESSAGE_IMAGE_ATTRIBUTE } from './intake'
import MessageAttachment from './MessageAttachment.vue'
import MessageImage from './MessageImage.vue'
import { useMessageClipboard } from './message-clipboard'
import { followMessageLink } from './message-links'

/**
 * A sent message as the conversation shows it (`composer.md` § Writing a
 * message): drawn once from the composer's schema, each node as the schema
 * draws it and each file as its capsule, with what the composer's editor
 * draws over its text (code's colors, bare URLs as links), and no editor
 * behind it. Its text selects as any text does; a copy or a drag of it, or
 * of a capsule, carries that part of the message in every form, its
 * formatting and its files with it, as the composer's own copy does: each
 * node it draws says where it stands in the message's document, so the
 * page's selection maps to the document's positions, and the document's
 * slice between them is what goes, as ProseMirror's own copy takes it.
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

const doc = computed(() => schema.nodeFromJSON(parseUserMarkdown(props.markdown, props.attachments)))
/** Where a drawn node stands in the message's document: the position before it. */
const POSITION = 'data-pos'

/** Whether a spec's entry is the element's attributes. */
function isAttributes(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

/** The element `spec` describes, with `content` in its hole; the schema draws every node and mark as an array. */
function fromSpec(spec: unknown, content: VNodeChild[], extra: Record<string, unknown> = {}): VNode {
  if (!Array.isArray(spec) || typeof spec[0] !== 'string') {
    throw new Error('A message node draws as an array spec.')
  }
  const rest: unknown[] = spec.slice(1)
  const attributes = isAttributes(rest[0]) ? rest[0] : {}
  const children = (isAttributes(rest[0]) ? rest.slice(1) : rest).flatMap((child): VNodeChild[] =>
    child === 0 ? content : Array.isArray(child) ? [fromSpec(child, content)] : [],
  )
  return h(spec[0], { ...attributes, ...extra }, children)
}

/** How the schema draws `mark`. */
function markSpec(mark: ProseMirrorNode['marks'][number]): DOMOutputSpec {
  const toDOM = mark.type.spec.toDOM
  if (!toDOM) {
    throw new Error(`The mark ${mark.type.name} has no drawing.`)
  }
  return toDOM(mark, true)
}

/** A text node, its bare URLs links, inside its marks as the schema draws them, the first mark outermost. */
function drawText(node: ProseMirrorNode): VNodeChild {
  const text = node.text ?? ''
  const parts: VNodeChild[] = []
  let at = 0
  for (const link of bareLinks(node)) {
    parts.push(text.slice(at, link.from), h('a', link.attributes, text.slice(link.from, link.to)))
    at = link.to
  }
  parts.push(text.slice(at))
  return [...node.marks].reverse().reduce<VNodeChild[]>(
    (inner, mark) => [fromSpec(markSpec(mark), inner)],
    parts.filter((part) => part !== ''),
  )
}

/** A code block's text in its colors. */
function drawCode(node: ProseMirrorNode, colors: ReturnType<typeof codeColors>): VNodeChild[] {
  const text = node.textContent
  const parts: VNodeChild[] = []
  let at = 0
  for (const run of colors.runs) {
    parts.push(text.slice(at, run.from), h('span', { class: run.classes }, text.slice(run.from, run.to)))
    at = run.to
  }
  parts.push(text.slice(at))
  return parts.filter((part) => part !== '')
}

/** `node`, which stands at `pos` in the message's document, drawn; each element of a node says where it stands. */
function draw(node: ProseMirrorNode, pos: number): VNodeChild {
  if (node.isText) {
    return drawText(node)
  }
  const at = { [POSITION]: String(pos) }
  switch (node.type.name) {
    case 'attachment':
      return h('span', { ...at, 'data-attachment': '', draggable: 'true' }, [
        h(MessageAttachment, { saved: node.attrs['capsule'] ?? null, editable: false }),
      ])
    case 'image': {
      const image = h('span', { ...at, [MESSAGE_IMAGE_ATTRIBUTE]: '', draggable: 'true' }, [h(MessageImage, {
        target: String(node.attrs['src'] ?? ''),
        alt: String(node.attrs['alt'] ?? ''),
        linked: node.marks.some((mark) => mark.type.name === 'link'),
        editable: false,
      })])
      return [...node.marks].reverse().reduce<VNodeChild>((inner, mark) => fromSpec(markSpec(mark), [inner]), image)
    }
  }
  const content: VNodeChild[] = []
  // The document's children start at 0; any other node's just inside it.
  const start = node.type.name === 'doc' ? 0 : pos + 1
  node.forEach((child, offset) => {
    content.push(draw(child, start + offset))
  })
  if (node.type.name === 'doc') {
    return content
  }
  const toDOM = node.type.spec.toDOM
  if (!toDOM) {
    throw new Error(`The node ${node.type.name} has no drawing.`)
  }
  const spec = toDOM(node)
  if (node.type.spec.code) {
    const colors = codeColors(node)
    return fromSpec(spec, drawCode(node, colors), { ...at, style: colors.style })
  }
  // An empty line, or one that ends in a break, holds a line's height, as the composer's editor gives it.
  if (node.isTextblock && (node.childCount === 0 || node.lastChild?.type.name === 'hardBreak')) {
    content.push(h('br'))
  }
  return fromSpec(spec, content, at)
}

/** The message drawn, again as the highlighter's colors arrive or the page's mode changes. */
function Content(): VNodeChild {
  void renderVersion.value
  return draw(doc.value, 0)
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

/** The part of the message `range` selects, as the document's slice between its ends. */
function selected(range: Range): Slice | null {
  const from = positionAt(range.startContainer, range.startOffset, 'start')
  const to = positionAt(range.endContainer, range.endOffset, 'end')
  return to > from ? doc.value.slice(from, to) : null
}

/** The page's selection when it lies within this message, and is not a part of one code block. */
function ownSelection(): Range | null {
  const selection = document.getSelection()
  const element = root.value
  if (!selection || selection.rangeCount === 0 || selection.isCollapsed || !element) {
    return null
  }
  const range = selection.getRangeAt(0)
  // A selection that runs on past the message is the page's to copy, whole and in order.
  if (!element.contains(range.startContainer) || !element.contains(range.endContainer)) {
    return null
  }
  const code = (node: Node) => (node instanceof Element ? node : node.parentElement)?.closest('pre') ?? null
  // Code copied from inside a code block is the code, as any page copies it.
  const block = code(range.startContainer)
  return block && block === code(range.endContainer) ? null : range
}

/** A selection in a sent message copies in every form; a lone picture also as itself. */
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
      .catch((error: unknown) => reportError(t('attachments.couldntCopyPicture'), error))
  }
}

/**
 * A drag of a capsule, or of the selected text, carries the message in
 * every form, its files with it, so a composer brings them back as capsules
 * without uploading them (`composer.md` § Attachments).
 */
function dragStart(event: DragEvent): void {
  const transfer = event.dataTransfer
  const host = event.target instanceof Element ? event.target.closest(`[data-attachment], [${MESSAGE_IMAGE_ATTRIBUTE}]`) : null
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
