<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, shallowRef, watch } from 'vue'
import { MessageCircleQuestionMark, MessageSquare, MessageSquarePlus } from '@lucide/vue'
import type { MediaSource, MessageComment } from '@demicodes/protocol'
import Popover from '../../ui/Popover.vue'
import SendField from '../../ui/SendField.vue'
import Tooltip from '../../ui/Tooltip.vue'
import { appOverlayStore } from '../../overlay/appOverlay'
import { usePhone } from '../../ui/phone'
import { readSelection, selectedText, type ContextMenuEntry, type ResolvedSelection } from '../../ui/context-menu'
import { t } from '../locales/text'
import { reportError } from '../../infra/errors'
import { anchorOf, liveOf, pictureAnchor, pictureOf, pictureQuote, rangeHas, rangeOf, staticOf, type TextAnchor } from './anchors'
import { pictureSource, useKeepPicture } from './pictures'
import CommentField from './CommentField.vue'
import type { MarkedComment, ReplyCommenting } from './comments'
import { publishMarks, type MarkKind } from './highlights'
import CommentItem from './CommentItem.vue'
import AskCard from './AskCard.vue'
import { cardContent, pageToRead, readKey, type ReplyAsking, type ShownSideQuestion } from './asks'

/**
 * The comments and side questions on a transcript's replies, drawn over the
 * list `root` holds.
 *
 * A comment is drawn as:
 *
 * - its text tinted and underlined, a sent one dimmer
 * - a comment mark in the reply's right margin at its line
 * - the card that shows a comment once its words or its mark are clicked
 * - the box a new one is written in under its selection
 *
 * A side question is drawn as its words in purple, and its card set into the
 * reply after the block holding the words' end, folded into a purple mark in
 * the margin.
 *
 * A comment or a side question on a picture outlines the picture in its color,
 * its margin mark at the picture's top.
 *
 * A reply is found by its text block's id (`data-reply-text`), so the marks
 * follow the rows the list draws as it scrolls. A row it does not draw shows
 * none.
 */
const props = defineProps<{
  /** The element the list's rows stand in, which this layer covers. */
  root: HTMLElement | null
  commenting: ReplyCommenting
  /**
   * The comments the transcript's messages sent, which show dimmer and take no
   * edit.
   */
  sent: readonly MessageComment[]
  /** The transcript's side questions; absent where it takes none. */
  asking?: ReplyAsking
}>()

/** Where something stands over the list, from its top left corner. */
interface Spot {
  top: number
  left: number
}

interface MarkerGroup {
  key: string
  top: number
  height: number
  comments: MarkedComment[]
  /** The folded side questions at the line. */
  asks: ShownSideQuestion[]
}

const BOX_WIDTH = 340
const CARD_WIDTH = 300

const phone = usePhone()
const width = ref(0)
/**
 * Too narrow for a margin: each line's marks collapse into a mark at its end.
 */
const narrow = computed(() => width.value > 0 && width.value < 640)

const marked = computed<MarkedComment[]>(() => [
  ...props.sent.map((comment) => ({ ...comment, sent: true })),
  ...props.commenting.waiting.map((comment) => ({ ...comment, sent: false })),
])

function replyRoot(blockId: string): HTMLElement | null {
  return props.root?.querySelector<HTMLElement>(`[data-reply-text="${CSS.escape(blockId)}"] .markdown-body`) ?? null
}

/**
 * What a comment, a side question or the box stands on: words, or a picture by
 * its place.
 */
type Placed = { blockId: string; quote: string; picture?: unknown } & TextAnchor

/**
 * The live range of `item` as its reply renders now: its words, or the picture
 * it names, whole.
 */
function liveRange(item: Placed): Range | null {
  const at = replyRoot(item.blockId)
  if (!at)
    return null
  if (!item.picture)
    return rangeOf(at, item)
  const image = pictureOf(at, item, item.quote)
  if (!image)
    return null
  const range = document.createRange()
  range.selectNode(image)
  return range
}

/**
 * `item`'s live range while the reply still holds what it quotes: its words, read
 * as the page reads a selection's (`selectedText`), or its picture, which
 * `liveRange` checks.
 */
function heldRange(item: Placed): Range | null {
  const range = liveRange(item)
  return range && (item.picture || selectedText(range) === item.quote) ? range : null
}

// --- The box under a selection, a new comment or a side question.

/**
 * The box: on words, or on a picture (`picture`), which it outlines as a
 * comment's.
 */
const box = shallowRef<{ kind: 'comment' | 'ask'; blockId: string; quote: string; picture: boolean; at: number } & TextAnchor | null>(null)
const boxText = ref('')
/**
 * While the box's picture is being kept: the box stays open with its text
 * meanwhile, and after a failure.
 */
const boxBusy = ref(false)
const keepPicture = useKeepPicture()

/**
 * Opens the box on `anchor`, whose `at` is where it stands in the block's
 * reading order.
 */
function openBox(kind: 'comment' | 'ask', blockId: string, anchor: TextAnchor & { at: number }, quote: string, picture = false): void {
  window.getSelection()?.removeAllRanges()
  closeCard()
  boxText.value = ''
  box.value = { kind, blockId, quote, picture, ...anchor }
}

/**
 * Sends what the box holds: a side question, or a comment, which waits with
 * the others, or, by Steer, goes alone at once. A picture goes as where
 * its bytes are, which for a picture of the web the page keeps first.
 */
async function sendBox(text: string, now = false): Promise<void> {
  const opened = box.value
  if (!opened || boxBusy.value)
    return
  const placed = { blockId: opened.blockId, start: opened.start, end: opened.end, quote: opened.quote }
  let picture: MediaSource | null = null
  if (opened.picture) {
    // The box keeps its text until the picture is kept; a failure leaves both,
    // and says why.
    boxBusy.value = true
    try {
      const at = replyRoot(opened.blockId)
      const image = at ? pictureOf(at, opened, opened.quote) : null
      if (!image)
        throw new Error('The picture is no longer in the reply.')
      picture = await pictureSource(image, keepPicture)
    } catch (error) {
      // A failure not about what was typed is a toast; the box keeps the text
      // to send again.
      if (box.value === opened)
        reportError(t(opened.kind === 'ask' ? 'ask.pictureFailed' : 'comments.pictureFailed'), error, { userVisible: true })
      return
    } finally {
      boxBusy.value = false
    }
    // Closed or opened elsewhere meanwhile: what it held is gone.
    if (box.value !== opened)
      return
  }
  box.value = null
  boxText.value = ''
  const pictured = picture ? { picture } : {}
  if (opened.kind === 'ask') {
    props.asking?.ask({ id: crypto.randomUUID(), ...placed, ...pictured, question: text })
    return
  }
  const comment = { id: crypto.randomUUID(), ...placed, at: opened.at, ...pictured, text }
  if (now && props.commenting.sendNow)
    props.commenting.sendNow(comment)
  else
    props.commenting.add(comment)
}

/**
 * Comment and Ask for `selected`, the reply's own text the page's selection takes
 * as the page read it (`readSelection`), in the reply `reply`, the text block
 * `blockId` whose rendered text is `root`: the comment quotes its text, as Copy
 * copies it. They are disabled while the reply is still being written, read where
 * the tool is drawn, so they turn enabled as the reply ends under a selection that
 * stays. None for a selection outside the reply's rendered text.
 */
function selectionTools(reply: { blockId: string; root: HTMLElement }, selected: ResolvedSelection, streaming: () => boolean): ContextMenuEntry[] {
  const at = reply.root.querySelector<HTMLElement>('.markdown-body')
  const anchor = at ? anchorOf(at, selected.range) : null
  if (!at || !anchor)
    return []
  const quote = selected.text
  // Read where the tool is drawn: it turns enabled as the reply ends under a
  // selection that stays.
  const tool = (kind: 'comment' | 'ask') => ({
    icon: kind === 'comment' ? MessageSquarePlus : MessageCircleQuestionMark,
    label: kind === 'comment' ? t('comments.comment') : t('ask.ask'),
    inPlace: true,
    get disabled() {
      return streaming()
    },
    disabledReason: t('comments.waitReply'),
    run: () => openBox(kind, reply.blockId, { ...anchor, at: anchor.start }, quote),
  })
  return props.asking ? [tool('comment'), tool('ask')] : [tool('comment')]
}

/**
 * Comment and Ask for the picture `image` of the reply `reply`, the text block
 * `blockId` whose rendered text is `root`. They are disabled while the reply is
 * being written, as for its words. None for a picture the reply does not hold,
 * as one in a side question's card.
 */
function pictureTools(reply: { blockId: string; root: HTMLElement }, image: HTMLImageElement, streaming: () => boolean): ContextMenuEntry[] {
  const at = reply.root.querySelector<HTMLElement>('.markdown-body')
  const anchor = at ? pictureAnchor(at, image) : null
  if (!anchor)
    return []
  const quote = pictureQuote(image)
  const tool = (kind: 'comment' | 'ask') => ({
    icon: kind === 'comment' ? MessageSquarePlus : MessageCircleQuestionMark,
    label: kind === 'comment' ? t('comments.comment') : t('ask.ask'),
    get disabled() {
      return streaming()
    },
    disabledReason: t('comments.waitReply'),
    run: () => openBox(kind, reply.blockId, anchor, quote, true),
  })
  return props.asking ? [tool('comment'), tool('ask')] : [tool('comment')]
}

defineExpose({ selectionTools, pictureTools })

// --- The card: the comments whose words or mark were clicked. The pointer
// resting on them shows nothing, as a comment in a document editor does not.

const card = shallowRef<{ ids: string[]; spot: Spot } | null>(null)
const cardComments = computed(() => marked.value.filter((comment) => card.value?.ids.includes(comment.id)))
const cardEl = ref<HTMLElement>()

function showCard(ids: string[], spot: Spot): void {
  card.value = { ids, spot }
}

function closeCard(): void {
  card.value = null
}

/** A line's comments as a sheet, on a phone. */
const sheet = shallowRef<{ ids: string[]; anchor: HTMLElement } | null>(null)
const sheetComments = computed(() => marked.value.filter((comment) => sheet.value?.ids.includes(comment.id)))

function remove(id: string): void {
  props.commenting.remove(id)
  if (card.value) {
    const ids = card.value.ids.filter((each) => each !== id)
    card.value = ids.length > 0 ? { ...card.value, ids } : null
  }
  if (sheet.value) {
    const ids = sheet.value.ids.filter((each) => each !== id)
    sheet.value = ids.length > 0 ? { ...sheet.value, ids } : null
  }
}

// --- Side questions: each card in the reply, folded into the margin's mark, or
// opened again.

const questions = computed(() => props.asking?.questions ?? [])
/**
 * Each card's place in its reply, an element set after the block holding the
 * end of its words.
 */
const hosts = new Map<string, HTMLElement>()
/** The side question under the pointer, whose words deepen. */
const hoveredAsk = ref<string | null>(null)
/** Each card's Reply field, open or not, and what it holds. */
const replies = ref<Record<string, { open: boolean; text: string }>>({})
/** The reads of the cards' transcripts asked for, each once (`readKey`). */
const asked = new Set<string>()

function replyOf(id: string): { open: boolean; text: string } {
  return (replies.value[id] ??= { open: false, text: '' })
}

function hostOf(id: string): HTMLElement {
  let host = hosts.get(id)
  if (!host) {
    host = document.createElement('div')
    host.dataset.askHost = ''
    // The card's words are not the reply's, which anchors count.
    host.dataset.noAnchor = ''
    hosts.set(id, host)
  }
  return host
}

/**
 * The reply's block that holds the end of `range`: a direct child of its
 * rendered text.
 */
function blockOf(at: HTMLElement, range: Range): Element | null {
  let node: Node | null = range.endContainer
  while (node && node.parentNode !== at)
    node = node.parentNode
  return node instanceof Element ? node : null
}

/**
 * Sets each card after the block holding the end of its words, once its row is
 * drawn and unless it is folded. The cards of one block follow it in the order
 * they were asked. A card already in its place is not touched, so a measurement
 * that finds them placed changes nothing, and the list's mutation observer,
 * which measures again, comes to rest.
 */
function placeHosts(placed: readonly { question: ShownSideQuestion; range: Range | null }[]): void {
  const after = new Map<Element, HTMLElement[]>()
  for (const { question, range } of placed) {
    const host = hostOf(question.id)
    const at = replyRoot(question.blockId)
    const block = range && at ? blockOf(at, range) : null
    if (question.folded || !block) {
      if (host.isConnected)
        host.remove()
      continue
    }
    after.set(block, [...after.get(block) ?? [], host])
  }
  for (const [block, inOrder] of after) {
    let previous: Element = block
    for (const host of inOrder) {
      if (previous.nextElementSibling !== host)
        previous.after(host)
      previous = host
    }
  }
}

/** The nearest element above `element` that scrolls its content up and down. */
function scrollParent(element: HTMLElement | null): HTMLElement | null {
  for (let at = element?.parentElement ?? null; at; at = at.parentElement) {
    if (/(auto|scroll)/.test(getComputedStyle(at).overflowY))
      return at
  }
  return null
}

/**
 * Folds a card keeping the reader's line where it is. The asked line stays put
 * while it is in view; scrolled off above, it comes to the top of the view
 * rather than the text below jumping under the reader.
 */
async function fold(question: ShownSideQuestion): Promise<void> {
  const scroller = scrollParent(props.root)
  const view = scroller?.getBoundingClientRect()
  const before = liveRange(question)?.getBoundingClientRect().top
  props.asking?.fold(question.id, true)
  await nextTick()
  measure()
  const after = liveRange(question)?.getBoundingClientRect().top
  if (!scroller || !view || before === undefined || after === undefined)
    return
  const target = before < view.top ? view.top + 16 : before
  scroller.scrollTop += after - target
}

/**
 * The words or the margin's mark clicked: the card, open again, comes into view
 * and takes the focus.
 */
async function reveal(question: ShownSideQuestion): Promise<void> {
  if (question.folded)
    props.asking?.fold(question.id, false)
  await nextTick()
  measure()
  const card = hosts.get(question.id)?.querySelector<HTMLElement>('[data-ask-card]')
  card?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
  card?.focus({ preventScroll: true })
}

// Each card reads what it needs of its transcript, each first question and
// answer, a page at a time, each read asked once.
watch(questions, (shown) => {
  for (const question of shown) {
    const key = readKey(question)
    const page = pageToRead(question)
    if (key && page && !asked.has(key)) {
      asked.add(key)
      props.asking?.read(question.id, page)
    }
  }
}, { immediate: true })

// --- Layout: the marks, the margin's comment marks, the box's place.

const layer = ref<HTMLElement>()
const markers = shallowRef<MarkerGroup[]>([])
/**
 * The outline of each picture a comment, a side question or the box stands on,
 * over the list.
 */
const outlines = shallowRef<{ key: string; kind: MarkKind; top: number; left: number; width: number; height: number }[]>([])
const boxSpot = shallowRef<Spot | null>(null)
/** Each mark's live range, for the pointer to find it. */
/**
 * Each mark's words, static (`liveOf`): a move of the list in the page leaves
 * them where they are.
 */
let hits: { id: string; range: StaticRangeInit }[] = []
let askHits: { id: string; range: StaticRangeInit }[] = []
const owner = Symbol('reply-comments')

/**
 * Under the range's last line, its left edge kept inside the list for a box
 * `boxWidth` wide.
 */
function spotUnder(words: Range | StaticRangeInit, base: DOMRect, boxWidth: number): Spot {
  const range = liveOf(words) ?? document.createRange()
  const rects = [...range.getClientRects()]
  const first = rects[0] ?? range.getBoundingClientRect()
  const last = rects.at(-1) ?? first
  const room = base.width - Math.min(boxWidth, base.width - 16) - 8
  return { top: last.bottom - base.top + 6, left: Math.max(8, Math.min(first.left - base.left, room)) }
}

function measure(): void {
  const base = props.root?.getBoundingClientRect()
  if (!base)
    return
  width.value = base.width
  const ranges: Record<MarkKind, Range[]> = { 'comment': [], 'comment-sent': [], 'comment-active': [], 'comment-selection': [], 'ask': [], 'ask-active': [] }
  const nextHits: typeof hits = []
  const nextAskHits: typeof askHits = []
  const groups: MarkerGroup[] = []
  const line = (range: Range) => {
    const rects = [...range.getClientRects()].filter((rect) => rect.width > 0)
    const rect = (narrow.value ? rects.at(-1) : rects[0]) ?? range.getBoundingClientRect()
    return { top: rect.top - base.top, height: rect.height }
  }
  // A picture's mark stands at its top, a line's height.
  const at = (range: Range, picture: unknown) => {
    if (!picture)
      return line(range)
    return { top: range.getBoundingClientRect().top - base.top, height: 20 }
  }
  const nextOutlines: typeof outlines.value = []
  const outline = (key: string, kind: MarkKind, range: Range) => {
    const rect = range.getBoundingClientRect()
    nextOutlines.push({ key, kind, top: rect.top - base.top, left: rect.left - base.left, width: rect.width, height: rect.height })
  }
  const groupAt = (spot: { top: number; height: number }) => {
    let group = groups.find((each) => Math.abs(each.top - spot.top) < 6)
    if (!group) {
      group = { key: `${Math.round(spot.top)}`, ...spot, comments: [], asks: [] }
      groups.push(group)
    }
    return group
  }
  const open = card.value?.ids ?? sheet.value?.ids ?? []
  for (const comment of marked.value) {
    // A block that no longer holds the text or the picture it quotes shows no
    // mark.
    const range = heldRange(comment)
    if (!range)
      continue
    const kind = open.includes(comment.id) ? 'comment-active' : comment.sent ? 'comment-sent' : 'comment'
    if (comment.picture)
      outline(comment.id, kind, range)
    else
      ranges[kind].push(range)
    nextHits.push({ id: comment.id, range: staticOf(range) })
    groupAt(at(range, comment.picture)).comments.push(comment)
  }
  // As a comment's: words or a picture the block no longer holds show no mark,
  // and no card.
  const placed = questions.value.map((question) => ({ question, range: heldRange(question) }))
  placeHosts(placed)
  for (const { question, range } of placed) {
    if (!range)
      continue
    const kind = question.id === hoveredAsk.value ? 'ask-active' : 'ask'
    if (question.picture)
      outline(question.id, kind, range)
    else
      ranges[kind].push(range)
    nextAskHits.push({ id: question.id, range: staticOf(range) })
    if (question.folded)
      groupAt(at(range, question.picture)).asks.push(question)
  }
  const boxed = box.value ? liveRange(box.value) : null
  if (boxed && box.value?.picture)
    outline('box', 'comment-selection', boxed)
  else if (boxed)
    ranges['comment-selection'].push(boxed)
  boxSpot.value = boxed ? spotUnder(boxed, base, BOX_WIDTH) : null
  hits = nextHits
  askHits = nextAskHits
  markers.value = groups.sort((a, b) => a.top - b.top)
  outlines.value = nextOutlines
  publishMarks(owner, ranges)
}

let frame = 0
function schedule(): void {
  if (frame === 0) {
    frame = requestAnimationFrame(() => {
      frame = 0
      measure()
    })
  }
}

// --- The pointer on the marks.

const pointing = ref(false)

function hitsAt(x: number, y: number) {
  return hits.filter((hit) => rangeHas(hit.range, x, y))
}

function askAt(x: number, y: number): ShownSideQuestion | undefined {
  const hit = askHits.find((each) => rangeHas(each.range, x, y))
  return hit && questions.value.find((question) => question.id === hit.id)
}

function onPointerMove(event: PointerEvent): void {
  if (event.pointerType === 'touch' || (event.target as Element).closest('[data-reply-comments] > *, [data-ask-card]'))
    return
  const asking = askAt(event.clientX, event.clientY)
  hoveredAsk.value = asking?.id ?? null
  pointing.value = asking !== undefined
}

function onPointerLeave(): void {
  pointing.value = false
  hoveredAsk.value = null
}

function onClick(event: MouseEvent): void {
  // A picture's click opens it in the viewer, as anywhere; its card shows under
  // the pointer and from its mark.
  if (readSelection() !== null || (event.target as Element).closest('[data-reply-comments] > *, [data-ask-card], a, button, img'))
    return
  const asking = askAt(event.clientX, event.clientY)
  if (asking) {
    void reveal(asking)
    return
  }
  const over = hitsAt(event.clientX, event.clientY)
  if (over.length === 0)
    return
  const ids = over.map((hit) => hit.id)
  if (phone.value)
    sheet.value = { ids, anchor: event.target as HTMLElement }
  else
    showCard(ids, spotUnder(over[0]!.range, props.root!.getBoundingClientRect(), CARD_WIDTH))
}

/** The card closes when a press lands outside it and its text. */
function onDocumentPointerDown(event: PointerEvent): void {
  const target = event.target as Element
  if (card.value && !cardEl.value?.contains(target) && !target.closest('[data-comment-marker]')
    && hitsAt(event.clientX, event.clientY).length === 0)
    closeCard()
}

function onDocumentKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape' && !event.defaultPrevented && card.value)
    closeCard()
}

function markerSpot(event: MouseEvent): Spot {
  const base = props.root!.getBoundingClientRect()
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  return { top: rect.bottom - base.top + 4, left: Math.max(8, rect.right - base.left - CARD_WIDTH) }
}

function onMarker(group: MarkerGroup, event: MouseEvent): void {
  const ids = group.comments.map((comment) => comment.id)
  if (phone.value)
    sheet.value = { ids, anchor: event.currentTarget as HTMLElement }
  else
    showCard(ids, markerSpot(event))
}

// The root comes and goes with the list: everything that follows it, its
// rows' changes and sizes and the pointer on it, is released with it.
watch(() => props.root, (root, _before, onCleanup) => {
  if (!root)
    return
  const listeners = new AbortController()
  // This layer stands in the root too: its own changes move no reply.
  const mutations = new MutationObserver((records) => {
    if (records.some((record) => !layer.value?.contains(record.target)))
      schedule()
  })
  // A row the list moves changes its style; a reply that streams, its text.
  mutations.observe(root, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ['style'] })
  const sizes = new ResizeObserver(schedule)
  sizes.observe(root)
  root.addEventListener('pointermove', onPointerMove, { signal: listeners.signal })
  root.addEventListener('pointerleave', onPointerLeave, { signal: listeners.signal })
  root.addEventListener('click', onClick, { signal: listeners.signal })
  document.addEventListener('pointerdown', onDocumentPointerDown, { capture: true, signal: listeners.signal })
  document.addEventListener('keydown', onDocumentKeydown, { signal: listeners.signal })
  schedule()
  onCleanup(() => {
    listeners.abort()
    mutations.disconnect()
    sizes.disconnect()
  })
}, { immediate: true })

watch([marked, box, card, sheet, narrow, questions, hoveredAsk], schedule)
watch(pointing, (now) => props.root?.classList.toggle('cursor-pointer', now))

onBeforeUnmount(() => {
  cancelAnimationFrame(frame)
  publishMarks(owner, null)
  for (const host of hosts.values())
    host.remove()
  hosts.clear()
})

// A side question that left the list, as a branch switch hides it, takes its
// place in the reply with it.
watch(questions, (shown) => {
  for (const [id, host] of hosts) {
    if (!shown.some((question) => question.id === id)) {
      host.remove()
      hosts.delete(id)
    }
  }
})

const countLabel = (count: number) => t('comments.count', { count }, count)
</script>

<template>
  <!--
    What stands over the replies: the margin's comment marks, the card and the
    box.
  -->
  <div ref="layer" data-reply-comments class="pointer-events-none absolute inset-0 z-20">
    <div
      v-for="frame in outlines"
      :key="frame.key"
      :data-picture-outline="frame.kind"
      class="picture-outline absolute"
      :class="`picture-outline-${frame.kind}`"
      :style="{ top: `${frame.top}px`, left: `${frame.left}px`, width: `${frame.width}px`, height: `${frame.height}px` }"
    />
    <div
      v-for="group in markers"
      :key="group.key"
      class="pointer-events-auto absolute flex items-center gap-0.5"
      :class="narrow ? 'right-0.5' : 'right-1.5'"
      :style="{ top: `${group.top + group.height / 2 - 10}px` }"
    >
      <button
        v-if="group.comments.length > 0"
        type="button"
        data-comment-marker
        class="comment-marker inline-flex h-5 min-w-5 items-center justify-center gap-0.5 rounded-full px-1 text-[11px] leading-none"
        :class="group.comments.some((comment) => !comment.sent) ? '' : 'comment-marker-sent'"
        :aria-label="countLabel(group.comments.length)"
        @click="onMarker(group, $event)"
      >
        <MessageSquare :size="12" aria-hidden="true" />
        <span v-if="group.comments.length > 1 && !narrow">{{ group.comments.length }}</span>
      </button>
      <Tooltip v-for="question in group.asks" :key="question.id" :content="t('ask.expand')">
        <button
          type="button"
          data-ask-marker
          class="ask-marker inline-flex h-5 min-w-5 items-center justify-center rounded-full px-1 text-[11px] leading-none"
          :aria-label="t('ask.expand')"
          @click="reveal(question)"
        >
          <MessageCircleQuestionMark :size="12" aria-hidden="true" />
        </button>
      </Tooltip>
    </div>

    <div
      v-if="card && cardComments.length > 0"
      ref="cardEl"
      data-comment-card
      class="overlay-shell pointer-events-auto absolute flex flex-col gap-1 rounded-lg px-2.5 py-1.5"
      :style="{ top: `${card.spot.top}px`, left: `${card.spot.left}px`, width: `min(${CARD_WIDTH}px, calc(100% - 16px))` }"
    >
      <CommentItem
        v-for="comment in cardComments"
        :key="comment.id"
        :text="comment.text"
        :sent="comment.sent"
        @save="commenting.edit(comment.id, $event)"
        @delete="remove(comment.id)"
      />
    </div>

    <div
      v-if="box && boxSpot"
      class="pointer-events-auto absolute"
      :style="{ top: `${boxSpot.top}px`, left: `${boxSpot.left}px`, width: `min(${BOX_WIDTH}px, calc(100% - 16px))` }"
    >
      <CommentField
        v-if="box.kind === 'comment'"
        :key="`${box.blockId}-${box.start}`"
        v-model="boxText"
        :send-now="commenting.sendNow !== undefined"
        :busy="boxBusy"
        @comment="sendBox($event)"
        @send-now="sendBox($event, true)"
        @cancel="box = null"
      />
      <SendField
        v-else
        :key="`${box.blockId}-${box.start}`"
        v-model="boxText"
        floating
        focus
        :holding="boxBusy"
        :placeholder="t('ask.placeholder')"
        :send-label="t('ask.ask')"
        @send="sendBox($event)"
        @cancel="box = null"
      />
    </div>
  </div>

  <!-- Each side question's card, in its place in the reply. -->
  <template v-if="asking">
    <Teleport v-for="question in questions" :key="question.id" :to="hostOf(question.id)">
      <AskCard
        v-if="!question.folded"
        :title="question.title"
        :turns="cardContent(question).turns"
        :failure="cardContent(question).failure"
        :failure-facts="question.failures[cardContent(question).failure?.id ?? ''] ?? null"
        :replying="replyOf(question.id).open"
        :reply="replyOf(question.id).text"
        @update:replying="replyOf(question.id).open = $event"
        @update:reply="replyOf(question.id).text = $event"
        @ask="asking.followUp(question.id, $event)"
        @retry="asking.retry(question.id)"
        @collapse="fold(question)"
        @open-full="asking.openFull?.(question.id)"
      />
    </Teleport>
  </template>

  <Popover
    v-if="sheet"
    is-open
    :overlay-store="appOverlayStore"
    :anchor-el="sheet.anchor"
    sheet
    @close="sheet = null"
  >
    <div class="flex w-full flex-col gap-3 p-3">
      <CommentItem
        v-for="comment in sheetComments"
        :key="comment.id"
        :text="comment.text"
        :sent="comment.sent"
        :quote="comment.quote"
        @save="commenting.edit(comment.id, $event)"
        @delete="remove(comment.id)"
      />
    </div>
  </Popover>
</template>
