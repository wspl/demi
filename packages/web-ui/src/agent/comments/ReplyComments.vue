<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, shallowRef, watch } from 'vue'
import { MessageCircleQuestionMark, MessageSquare, MessageSquarePlus } from '@lucide/vue'
import type { MessageComment } from '@demicodes/protocol'
import Popover from '../../ui/Popover.vue'
import SendField from '../../ui/SendField.vue'
import Tooltip from '../../ui/Tooltip.vue'
import { appOverlayStore } from '../../overlay/appOverlay'
import { usePhone } from '../../ui/phone'
import type { ContextMenuEntry } from '../../ui/context-menu'
import { t } from '../locales/text'
import { anchorOf, liveOf, rangeHas, rangeOf, staticOf, type TextAnchor } from './anchors'
import type { MarkedComment, ReplyCommenting } from './comments'
import { publishMarks, type MarkKind } from './highlights'
import CommentItem from './CommentItem.vue'
import AskCard from './AskCard.vue'
import { cardContent, pageToRead, readKey, type ReplyAsking, type ShownSideQuestion } from './asks'

/**
 * The comments and side questions on a transcript's replies
 * (`comments-and-ask.md`), drawn over the list `root` holds: each comment's
 * text tinted and underlined, a sent one dimmer, a comment mark in the
 * reply's right margin at its line, the card that shows a comment under the
 * pointer or pinned by a click, and the box a new one is written in under
 * its selection; each side question's words in purple, its card set into
 * the reply after the block holding the words' end, folded into a purple
 * mark in the margin. A reply is found by its text block's id
 * (`data-reply-text`), so the marks follow the rows the list draws as it
 * scrolls; a row it does not draw shows none.
 */
const props = defineProps<{
  /** The element the list's rows stand in, which this layer covers. */
  root: HTMLElement | null
  commenting: ReplyCommenting
  /** The comments the transcript's messages sent, which show dimmer and take no edit. */
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
/** Too narrow for a margin: each line's marks collapse into a mark at its end. */
const narrow = computed(() => width.value > 0 && width.value < 640)

const marked = computed<MarkedComment[]>(() => [
  ...props.sent.map((comment) => ({ ...comment, sent: true })),
  ...props.commenting.waiting.map((comment) => ({ ...comment, sent: false })),
])

function replyRoot(blockId: string): HTMLElement | null {
  return props.root?.querySelector<HTMLElement>(`[data-reply-text="${CSS.escape(blockId)}"] .markdown-body`) ?? null
}

function liveRange(item: { blockId: string } & TextAnchor): Range | null {
  const at = replyRoot(item.blockId)
  return at ? rangeOf(at, item) : null
}

// --- The box under a selection, a new comment or a side question.

const box = shallowRef<{ kind: 'comment' | 'ask'; blockId: string; quote: string } & TextAnchor | null>(null)
const boxText = ref('')

function openBox(kind: 'comment' | 'ask', blockId: string, anchor: TextAnchor, quote: string): void {
  window.getSelection()?.removeAllRanges()
  closeCard()
  boxText.value = ''
  box.value = { kind, blockId, quote, ...anchor }
}

function sendBox(text: string): void {
  const opened = box.value
  box.value = null
  if (!opened)
    return
  const placed = { blockId: opened.blockId, start: opened.start, end: opened.end, quote: opened.quote }
  if (opened.kind === 'ask')
    props.asking?.ask({ id: crypto.randomUUID(), ...placed, question: text })
  else
    props.commenting.add({ id: crypto.randomUUID(), ...placed, text })
}

/**
 * Comment for the selection in the reply `reply`, the text block `blockId`
 * whose rendered text is `root`; disabled while the reply is still being
 * written, read where the tool is drawn, so it turns enabled as the reply
 * ends under a selection that stays. None for a selection the reply does
 * not hold.
 */
function selectionTools(reply: { blockId: string; root: HTMLElement }, streaming: () => boolean): ContextMenuEntry[] {
  const selection = window.getSelection()
  if (!selection || selection.rangeCount === 0)
    return []
  const range = selection.getRangeAt(0)
  const at = reply.root.querySelector<HTMLElement>('.markdown-body')
  // A selection reaching into a side question's card is the card's, which asks nothing again.
  if (at && [...at.querySelectorAll('[data-ask-host]')].some((host) => range.intersectsNode(host)))
    return []
  const anchor = at ? anchorOf(at, range) : null
  if (!at || !anchor)
    return []
  // The quote is read as the mark is checked against it (`measure`), so the two agree.
  const quote = rangeOf(at, anchor)?.toString() ?? range.toString()
  // Read where the tool is drawn: it turns enabled as the reply ends under a selection that stays.
  const tool = (kind: 'comment' | 'ask') => ({
    icon: kind === 'comment' ? MessageSquarePlus : MessageCircleQuestionMark,
    label: kind === 'comment' ? t('comments.comment') : t('ask.ask'),
    inPlace: true,
    get disabled() {
      return streaming()
    },
    disabledReason: t('comments.waitReply'),
    run: () => openBox(kind, reply.blockId, anchor, quote),
  })
  return props.asking ? [tool('comment'), tool('ask')] : [tool('comment')]
}

defineExpose({ selectionTools })

// --- The card: the comments under the pointer, or pinned open by a click.

const card = shallowRef<{ ids: string[]; spot: Spot; pinned: boolean } | null>(null)
const cardComments = computed(() => marked.value.filter((comment) => card.value?.ids.includes(comment.id)))
const cardEl = ref<HTMLElement>()
let hideTimer: ReturnType<typeof setTimeout> | undefined

function showCard(ids: string[], spot: Spot, pinned: boolean): void {
  clearTimeout(hideTimer)
  if (card.value?.pinned && !pinned)
    return
  if (!pinned && card.value?.ids.join() === ids.join())
    return
  card.value = { ids, spot, pinned }
}

function leaveCard(): void {
  clearTimeout(hideTimer)
  if (card.value && !card.value.pinned)
    hideTimer = setTimeout(() => (card.value = null), 160)
}

function closeCard(): void {
  clearTimeout(hideTimer)
  card.value = null
}

function pinCard(): void {
  if (card.value)
    card.value = { ...card.value, pinned: true }
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

// --- Side questions: each card in the reply, folded into the margin's mark, or opened again.

const questions = computed(() => props.asking?.questions ?? [])
/** Each card's place in its reply, an element set after the block holding the end of its words. */
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

/** The reply's block that holds the end of `range`: a direct child of its rendered text. */
function blockOf(at: HTMLElement, range: Range): Element | null {
  let node: Node | null = range.endContainer
  while (node && node.parentNode !== at)
    node = node.parentNode
  return node instanceof Element ? node : null
}

/**
 * Sets each card after the block holding the end of its words, once its row
 * is drawn and unless it is folded: the cards of one block follow it in the
 * order they were asked. A card already in its place is not touched, so a
 * measurement that finds them placed changes nothing, and the list's
 * mutation observer, which measures again, comes to rest.
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
 * Folds a card keeping the reader's line where it is: the asked line stays
 * put while it is in view; scrolled off above, it comes to the top of the
 * view rather than the text below jumping under the reader.
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

/** The words or the margin's mark clicked: the card, open again, comes into view and takes the focus. */
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
const boxSpot = shallowRef<Spot | null>(null)
/** Each mark's live range, for the pointer to find it. */
/** Each mark's words, static (`liveOf`): a move of the list in the page leaves them where they are. */
let hits: { id: string; range: StaticRangeInit }[] = []
let askHits: { id: string; range: StaticRangeInit }[] = []
const owner = Symbol('reply-comments')

/** Under the range's last line, its left edge kept inside the list for a box `boxWidth` wide. */
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
    const range = liveRange(comment)
    // A block that no longer holds the text it quotes shows no mark.
    if (!range || range.toString() !== comment.quote)
      continue
    ranges[open.includes(comment.id) ? 'comment-active' : comment.sent ? 'comment-sent' : 'comment'].push(range)
    nextHits.push({ id: comment.id, range: staticOf(range) })
    groupAt(line(range)).comments.push(comment)
  }
  const placed = questions.value.map((question) => {
    const live = liveRange(question)
    // As a comment's: words the block no longer holds show no mark, and no card.
    return { question, range: live && live.toString() === question.quote ? live : null }
  })
  placeHosts(placed)
  for (const { question, range } of placed) {
    if (!range)
      continue
    ranges[question.id === hoveredAsk.value ? 'ask-active' : 'ask'].push(range)
    nextAskHits.push({ id: question.id, range: staticOf(range) })
    if (question.folded)
      groupAt(line(range)).asks.push(question)
  }
  const boxed = box.value ? liveRange(box.value) : null
  if (boxed)
    ranges['comment-selection'].push(boxed)
  boxSpot.value = boxed ? spotUnder(boxed, base, BOX_WIDTH) : null
  hits = nextHits
  askHits = nextAskHits
  markers.value = groups.sort((a, b) => a.top - b.top)
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
  const over = hitsAt(event.clientX, event.clientY)
  const asking = askAt(event.clientX, event.clientY)
  hoveredAsk.value = asking?.id ?? null
  pointing.value = over.length > 0 || asking !== undefined
  if (over.length === 0) {
    leaveCard()
    return
  }
  showCard(over.map((hit) => hit.id), spotUnder(over[0]!.range, props.root!.getBoundingClientRect(), CARD_WIDTH), false)
}

function onPointerLeave(): void {
  pointing.value = false
  hoveredAsk.value = null
  leaveCard()
}

function onClick(event: MouseEvent): void {
  if (!window.getSelection()?.isCollapsed || (event.target as Element).closest('[data-reply-comments] > *, [data-ask-card], a, button'))
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
    showCard(ids, spotUnder(over[0]!.range, props.root!.getBoundingClientRect(), CARD_WIDTH), true)
}

/** A pinned card closes when a press lands outside it and its text. */
function onDocumentPointerDown(event: PointerEvent): void {
  const target = event.target as Element
  if (card.value?.pinned && !cardEl.value?.contains(target) && !target.closest('[data-comment-marker]')
    && hitsAt(event.clientX, event.clientY).length === 0)
    closeCard()
}

function onDocumentKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape' && !event.defaultPrevented && card.value?.pinned)
    closeCard()
}

function markerSpot(event: MouseEvent): Spot {
  const base = props.root!.getBoundingClientRect()
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  return { top: rect.bottom - base.top + 4, left: Math.max(8, rect.right - base.left - CARD_WIDTH) }
}

function onMarker(group: MarkerGroup, event: MouseEvent, pinned: boolean): void {
  const ids = group.comments.map((comment) => comment.id)
  if (phone.value) {
    if (pinned)
      sheet.value = { ids, anchor: event.currentTarget as HTMLElement }
    return
  }
  showCard(ids, markerSpot(event), pinned)
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
  clearTimeout(hideTimer)
  publishMarks(owner, null)
  for (const host of hosts.values())
    host.remove()
  hosts.clear()
})

// A side question that left the list, as a branch switch hides it, takes its place in the reply with it.
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
  <!-- What stands over the replies: the margin's comment marks, the card and the box. -->
  <div ref="layer" data-reply-comments class="pointer-events-none absolute inset-0 z-20">
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
        @pointerenter="(event: PointerEvent) => event.pointerType !== 'touch' && onMarker(group, event, false)"
        @pointerleave="leaveCard"
        @click="onMarker(group, $event, true)"
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
      @pointerenter="showCard(card.ids, card.spot, card.pinned)"
      @pointerleave="leaveCard"
    >
      <CommentItem
        v-for="comment in cardComments"
        :key="comment.id"
        :text="comment.text"
        :sent="comment.sent"
        @save="commenting.edit(comment.id, $event)"
        @delete="remove(comment.id)"
        @editing="$event && pinCard()"
      />
    </div>

    <div
      v-if="box && boxSpot"
      class="pointer-events-auto absolute"
      :style="{ top: `${boxSpot.top}px`, left: `${boxSpot.left}px`, width: `min(${BOX_WIDTH}px, calc(100% - 16px))` }"
    >
      <SendField
        :key="`${box.blockId}-${box.start}`"
        v-model="boxText"
        floating
        focus
        :placeholder="box.kind === 'ask' ? t('ask.placeholder') : t('comments.placeholder')"
        :send-label="box.kind === 'ask' ? t('ask.ask') : t('comments.comment')"
        @send="sendBox"
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
