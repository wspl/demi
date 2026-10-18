<script setup lang="ts">
import { computed, onBeforeUnmount, ref, shallowRef, watch } from 'vue'
import { MessageSquare, MessageSquarePlus } from '@lucide/vue'
import type { MessageComment } from '@demicodes/protocol'
import Popover from '../../ui/Popover.vue'
import SendField from '../../ui/SendField.vue'
import { appOverlayStore } from '../../overlay/appOverlay'
import { usePhone } from '../../ui/phone'
import type { ContextMenuEntry } from '../../ui/context-menu'
import { t } from '../locales/text'
import { anchorOf, rangeHas, rangeOf, type TextAnchor } from './anchors'
import type { MarkedComment, ReplyCommenting } from './comments'
import { publishMarks, type MarkKind } from './highlights'
import CommentItem from './CommentItem.vue'

/**
 * The comments on a transcript's replies (`comments-and-ask.md`), drawn
 * over the list `root` holds: each comment's text tinted and underlined,
 * a sent one dimmer, a comment mark in the reply's right margin at its
 * line, the card that shows a comment under the pointer or pinned by a
 * click, and the box a new one is written in under its selection. A reply
 * is found by its text block's id (`data-reply-text`), so the marks follow
 * the rows the list draws as it scrolls; a row it does not draw shows none.
 */
const props = defineProps<{
  /** The element the list's rows stand in, which this layer covers. */
  root: HTMLElement | null
  commenting: ReplyCommenting
  /** The comments the transcript's messages sent, which show dimmer and take no edit. */
  sent: readonly MessageComment[]
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

// --- The box under a selection, a new comment.

const box = shallowRef<{ blockId: string; quote: string } & TextAnchor | null>(null)
const boxText = ref('')

function openBox(blockId: string, anchor: TextAnchor, quote: string): void {
  window.getSelection()?.removeAllRanges()
  closeCard()
  boxText.value = ''
  box.value = { blockId, quote, ...anchor }
}

function sendBox(text: string): void {
  const opened = box.value
  box.value = null
  if (opened)
    props.commenting.add({ id: crypto.randomUUID(), blockId: opened.blockId, start: opened.start, end: opened.end, quote: opened.quote, text })
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
  const anchor = at ? anchorOf(at, range) : null
  if (!at || !anchor)
    return []
  // The quote is read as the mark is checked against it (`measure`), so the two agree.
  const quote = rangeOf(at, anchor)?.toString() ?? range.toString()
  return [{
    icon: MessageSquarePlus,
    label: t('comments.comment'),
    inPlace: true,
    get disabled() {
      return streaming()
    },
    disabledReason: t('comments.waitReply'),
    run: () => openBox(reply.blockId, anchor, quote),
  }]
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

// --- Layout: the marks, the margin's comment marks, the box's place.

const layer = ref<HTMLElement>()
const markers = shallowRef<MarkerGroup[]>([])
const boxSpot = shallowRef<Spot | null>(null)
/** Each mark's live range, for the pointer to find it. */
let hits: { id: string; range: Range }[] = []
const owner = Symbol('reply-comments')

/** Under the range's last line, its left edge kept inside the list for a box `boxWidth` wide. */
function spotUnder(range: Range, base: DOMRect, boxWidth: number): Spot {
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
  const ranges: Record<MarkKind, Range[]> = { 'comment': [], 'comment-sent': [], 'comment-active': [], 'comment-selection': [] }
  const nextHits: typeof hits = []
  const groups: MarkerGroup[] = []
  const line = (range: Range) => {
    const rects = [...range.getClientRects()].filter((rect) => rect.width > 0)
    const rect = (narrow.value ? rects.at(-1) : rects[0]) ?? range.getBoundingClientRect()
    return { top: rect.top - base.top, height: rect.height }
  }
  const groupAt = (spot: { top: number; height: number }) => {
    let group = groups.find((each) => Math.abs(each.top - spot.top) < 6)
    if (!group) {
      group = { key: `${Math.round(spot.top)}`, ...spot, comments: [] }
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
    nextHits.push({ id: comment.id, range })
    groupAt(line(range)).comments.push(comment)
  }
  const boxed = box.value ? liveRange(box.value) : null
  if (boxed)
    ranges['comment-selection'].push(boxed)
  boxSpot.value = boxed ? spotUnder(boxed, base, BOX_WIDTH) : null
  hits = nextHits
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

function onPointerMove(event: PointerEvent): void {
  if (event.pointerType === 'touch' || (event.target as Element).closest('[data-reply-comments] > *'))
    return
  const over = hitsAt(event.clientX, event.clientY)
  pointing.value = over.length > 0
  if (over.length === 0) {
    leaveCard()
    return
  }
  showCard(over.map((hit) => hit.id), spotUnder(over[0]!.range, props.root!.getBoundingClientRect(), CARD_WIDTH), false)
}

function onPointerLeave(): void {
  pointing.value = false
  leaveCard()
}

function onClick(event: MouseEvent): void {
  if (!window.getSelection()?.isCollapsed || (event.target as Element).closest('[data-reply-comments] > *, a, button'))
    return
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

watch([marked, box, card, sheet, narrow], schedule)
watch(pointing, (now) => props.root?.classList.toggle('cursor-pointer', now))

onBeforeUnmount(() => {
  cancelAnimationFrame(frame)
  clearTimeout(hideTimer)
  publishMarks(owner, null)
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
        :placeholder="t('comments.placeholder')"
        :send-label="t('comments.comment')"
        @send="sendBox"
        @cancel="box = null"
      />
    </div>
  </div>

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
