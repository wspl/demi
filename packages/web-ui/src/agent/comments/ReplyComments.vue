<script setup lang="ts">
import { keptComputed } from '../../infra/keep-same'
import { computed, onBeforeUnmount, ref, shallowRef, watch } from 'vue'
import { MessageCircleQuestionMark, MessageSquarePlus } from '@lucide/vue'
import type { MediaSource, MessageComment } from '@demicodes/protocol'
import { usePhone } from '../../ui/phone'
import { selectedText, type ContextMenuEntry, type ResolvedSelection, type SelectionFieldActions } from '../../ui/context-menu'
import { chooseWay, openPictureField } from './selection-field'
import { t } from '../locales/text'
import { reportError } from '../../infra/errors'
import { anchorOf, pictureAnchor, pictureOf, pictureQuote, rangeOf, type TextAnchor } from './anchors'
import { pictureSource, useKeepPicture } from './pictures'
import type { MarkedComment, ReplyCommenting } from './comments'
import { publishMarks, type MarkKind } from './highlights'
import { aroundPlaces, clearPlaces, createPlace, placeRemarks, PLACE_ATTRIBUTE, type Remark } from './lines'
import CommentItem from './CommentItem.vue'
import AskCard from './AskCard.vue'
import { cardContent, pageToRead, readKey, unanswered, type ReplyAsking } from './asks'
import { useComposerFocus } from '../composer-focus'

/**
 * The comments and side questions on a transcript's replies
 * (`docs/decisions/0092-a-remark-stands-under-its-line.md`).
 *
 * - A comment's words are tinted and underlined yellow, a sent one dimmer, and
 *   its box stands under the line they end on, its text always shown.
 * - A side question's words are purple, and its card stands under their line,
 *   open or folded to its header.
 * - A comment or a side question on a picture outlines the picture in its
 *   color, its box or card under the picture's line.
 *
 * Each box and card is teleported into a place of its own in the reply
 * (`lines.ts`). A reply is found by its text block's id (`data-reply-text`), so
 * the places follow the rows the list draws as it scrolls. A row it does not
 * draw shows none.
 *
 * A phone takes no remark on a selection or a picture: it shows the ones made
 * elsewhere.
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

const phone = usePhone()
const composer = useComposerFocus()

const marked = keptComputed<MarkedComment[]>(() => [
  ...props.sent.map((comment) => ({ ...comment, sent: true })),
  ...props.commenting.waiting.map((comment) => ({ ...comment, sent: false })),
])

function replyRoot(blockId: string): HTMLElement | null {
  return props.root?.querySelector<HTMLElement>(`[data-reply-text="${CSS.escape(blockId)}"] .markdown-body`) ?? null
}

/**
 * What a comment or a side question stands on: words, or a picture by
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

// --- What the page's one comment field does for a selection or a picture of a reply.

const keepPicture = useKeepPicture()

/**
 * Comment and Ask for `selected`, the reply's own text the page's selection takes
 * as the page read it (`readSelection`), in the reply `reply`, the text block
 * `blockId` whose rendered text is `root`: the comment quotes its text, as Copy
 * copies it. They are disabled while the reply is still being written, read where
 * the tool is drawn, so they turn enabled as the reply ends under a selection that
 * stays. None for a selection outside the reply's rendered text.
 */
function selectionTools(reply: { blockId: string; root: HTMLElement }, selected: ResolvedSelection, streaming: () => boolean): ContextMenuEntry[] {
  // A phone takes no remark on a selection or a picture.
  if (phone.value)
    return []
  const at = reply.root.querySelector<HTMLElement>('.markdown-body')
  const anchor = at ? anchorOf(at, selected.range) : null
  if (!at || !anchor)
    return []
  // Read where the tool is drawn: it turns enabled as the reply ends under a
  // selection that stays.
  const tool = (kind: 'comment' | 'ask') => ({
    icon: kind === 'comment' ? MessageSquarePlus : MessageCircleQuestionMark,
    label: kind === 'comment' ? t('comments.comment') : t('ask.ask'),
    get disabled() {
      return streaming()
    },
    disabledReason: t('comments.waitReply'),
    run: () => chooseWay(kind),
  })
  return props.asking ? [tool('comment'), tool('ask')] : [tool('comment')]
}

/**
 * Comment and Ask for the picture `image` of the reply `reply`, the text block
 * `blockId` whose rendered text is `root`: each opens the page's one comment
 * field on the picture. They are disabled while the reply is being written, as for
 * its words. None for a picture the reply does not hold, as one in a side
 * question's card.
 */
function pictureTools(reply: { blockId: string; root: HTMLElement }, image: HTMLImageElement, streaming: () => boolean): ContextMenuEntry[] {
  // A phone takes no remark on a selection or a picture.
  if (phone.value)
    return []
  const at = reply.root.querySelector<HTMLElement>('.markdown-body')
  const anchor = at ? pictureAnchor(at, image) : null
  if (!anchor)
    return []
  const actions = fieldActions(reply.blockId, anchor, pictureQuote(image), streaming, true)
  const tool = (kind: 'comment' | 'ask') => ({
    icon: kind === 'comment' ? MessageSquarePlus : MessageCircleQuestionMark,
    label: kind === 'comment' ? t('comments.comment') : t('ask.ask'),
    get disabled() {
      return streaming()
    },
    disabledReason: t('comments.waitReply'),
    run: () => openPictureField({ image, actions, way: kind }),
  })
  return props.asking ? [tool('comment'), tool('ask')] : [tool('comment')]
}

/**
 * What the page's one comment field does for the words or the picture `anchor`
 * names in the reply text block `blockId`, which `quote` quotes:
 *
 * - Comment waits with the other comments, and the composer takes the focus
 *   once the field has closed, so Return sends them.
 * - Ask asks a side question, where the list takes them.
 * - Send sends the comment alone now, where the list can.
 *
 * A picture goes as where its bytes are, which for a picture of the web the page
 * keeps first; a failure says why and leaves the field open with its words.
 */
function fieldActions(blockId: string, anchor: TextAnchor & { at: number }, quote: string, streaming: () => boolean, picture = false): SelectionFieldActions {
  const placed = { blockId, start: anchor.start, end: anchor.end, quote }
  const withPicture = (kind: 'comment' | 'ask', make: (kept: { picture?: MediaSource }) => void): void | Promise<void> => {
    if (!picture)
      return make({})
    return (async () => {
      try {
        const at = replyRoot(blockId)
        const image = at ? pictureOf(at, anchor, quote) : null
        if (!image)
          throw new Error('The picture is no longer in the reply.')
        make({ picture: await pictureSource(image, keepPicture) })
      } catch (error) {
        reportError(t(kind === 'ask' ? 'ask.pictureFailed' : 'comments.pictureFailed'), error, {
          site: kind === 'ask' ? 'ask.pictureFailed' : 'comments.pictureFailed',
          userVisible: true,
        })
        throw error
      }
    })()
  }
  const comment = (text: string, kept: { picture?: MediaSource }) => ({ id: crypto.randomUUID(), ...placed, at: anchor.at, ...kept, text })
  const { sendNow } = props.commenting
  return {
    comment: (words) => withPicture('comment', (kept) => props.commenting.add(comment(words, kept))),
    commented: () => composer.focus(),
    ask: props.asking ? (words) => withPicture('ask', (kept) => props.asking?.ask({ id: crypto.randomUUID(), ...placed, ...kept, question: words })) : undefined,
    send: sendNow ? (words) => withPicture('comment', (kept) => sendNow(comment(words, kept))) : undefined,
    get unavailable() {
      return streaming() ? t('comments.waitReply') : null
    },
  }
}

/**
 * What the page's one comment field does for `selected`, the reply's own text the
 * page's selection takes, in the reply `reply` (`fieldActions`). None for a
 * selection outside the reply's rendered text.
 */
function selectionField(reply: { blockId: string; root: HTMLElement }, selected: ResolvedSelection, streaming: () => boolean): SelectionFieldActions | null {
  if (phone.value)
    return null
  const at = reply.root.querySelector<HTMLElement>('.markdown-body')
  const anchor = at ? anchorOf(at, selected.range) : null
  if (!at || !anchor)
    return null
  return fieldActions(reply.blockId, { ...anchor, at: anchor.start }, selected.text, streaming)
}

defineExpose({ selectionTools, selectionField, pictureTools })

// --- Each box's and card's place in its reply.

/** The place of each comment (`comment:<id>`) and side question (`ask:<id>`). */
const places = new Map<string, HTMLElement>()

function placeOf(key: string): HTMLElement {
  let place = places.get(key)
  if (!place) {
    place = createPlace()
    places.set(key, place)
  }
  return place
}

/** Lets go of the places whose comment or side question went. */
function prune(keys: ReadonlySet<string>): void {
  for (const [key, place] of places) {
    if (!keys.has(key)) {
      place.remove()
      places.delete(key)
    }
  }
}

// --- Side questions: each card under its line, open or folded to its header.

const questions = keptComputed(() => props.asking?.questions ?? [])
/** Each card's Reply field, open or not, and what it holds. */
const replies = ref<Record<string, { open: boolean; text: string }>>({})
/** The reads of the cards' transcripts asked for, each once (`readKey`). */
const asked = new Set<string>()

function replyOf(id: string): { open: boolean; text: string } {
  return (replies.value[id] ??= { open: false, text: '' })
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

// --- Layout: the places, the marks and the pictures' outlines.

const layer = ref<HTMLElement>()
/**
 * The outline of each picture a comment or a side question stands on, over the
 * list.
 */
const outlines = shallowRef<{ key: string; kind: MarkKind; top: number; left: number; width: number; height: number }[]>([])
const owner = Symbol('reply-comments')
/** The list's width the places were set at: another wraps the lines again. */
let placedWidth = 0

function measure(): void {
  const base = props.root?.getBoundingClientRect()
  if (!base)
    return
  if (base.width !== placedWidth) {
    clearPlaces(places.values())
    placedWidth = base.width
  }
  const keys = new Set<string>()
  const remark = (key: string, item: Placed): Remark => {
    keys.add(key)
    return { place: placeOf(key), root: replyRoot(item.blockId), range: () => heldRange(item) }
  }
  const remarks = [
    ...marked.value.map((comment) => remark(`comment:${comment.id}`, comment)),
    ...questions.value.map((question) => remark(`ask:${question.id}`, question)),
  ]
  prune(keys)
  placeRemarks(remarks)
  // Read once the places are set: placing split and joined the words' text.
  const ranges: Record<MarkKind, Range[]> = { 'comment': [], 'comment-sent': [], 'ask': [] }
  const nextOutlines: typeof outlines.value = []
  const outline = (key: string, kind: MarkKind, range: Range) => {
    const rect = range.getBoundingClientRect()
    nextOutlines.push({ key, kind, top: rect.top - base.top, left: rect.left - base.left, width: rect.width, height: rect.height })
  }
  for (const comment of marked.value) {
    // A block that no longer holds the text or the picture it quotes shows no
    // mark, and no box.
    const range = heldRange(comment)
    if (!range)
      continue
    const kind = comment.sent ? 'comment-sent' : 'comment'
    if (comment.picture)
      outline(comment.id, kind, range)
    else
      ranges[kind].push(...aroundPlaces(range))
  }
  for (const question of questions.value) {
    const range = heldRange(question)
    if (!range)
      continue
    if (question.picture)
      outline(question.id, 'ask', range)
    else
      ranges.ask.push(...aroundPlaces(range))
  }
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

/**
 * Copy of a selection that crosses a box or a card copies the reply's words
 * alone, as the selection's menu does: the boxes stand in the text but are none
 * of it.
 */
function copyWords(event: ClipboardEvent): void {
  const selection = window.getSelection()
  if (!selection || selection.isCollapsed || selection.rangeCount === 0 || !event.clipboardData)
    return
  const range = selection.getRangeAt(0)
  const crossed = [...places.values()].filter((place) => place.isConnected && range.intersectsNode(place))
  if (crossed.length === 0 || crossed.some((place) => place.contains(range.commonAncestorContainer)))
    return
  const html = document.createElement('div')
  html.append(range.cloneContents())
  for (const place of html.querySelectorAll(`[${PLACE_ATTRIBUTE}]`))
    place.remove()
  event.clipboardData.setData('text/plain', selectedText(range))
  event.clipboardData.setData('text/html', html.innerHTML)
  event.preventDefault()
}

/** A change inside a box or a card, as an answer streaming, moves no words. */
function insidePlace(node: Node): boolean {
  const element = node instanceof Element ? node : node.parentElement
  return element?.closest(`[${PLACE_ATTRIBUTE}]`) != null
}

// The root comes and goes with the list: everything that follows it, its
// rows' changes and sizes, is released with it.
watch(() => props.root, (root, _before, onCleanup) => {
  if (!root)
    return
  root.addEventListener('copy', copyWords)
  placedWidth = 0
  onCleanup(() => root.removeEventListener('copy', copyWords))
}, { immediate: true })

/** Whether a comment or a side question stands on the replies, whose places follow the words. */
const placing = computed(() => marked.value.length > 0 || questions.value.length > 0)

// The words are followed only while something stands on them: a reply that
// streams with nothing to place is measured for nothing.
watch([() => props.root, placing], ([root, any], _before, onCleanup) => {
  if (!root || !any)
    return
  // This layer stands in the root too: its own changes move no reply.
  const mutations = new MutationObserver((records) => {
    if (records.some((record) => !layer.value?.contains(record.target) && !insidePlace(record.target)))
      schedule()
  })
  // A row the list moves changes its style; a reply that streams, its text.
  mutations.observe(root, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ['style'] })
  const sizes = new ResizeObserver(schedule)
  sizes.observe(root)
  schedule()
  onCleanup(() => {
    mutations.disconnect()
    sizes.disconnect()
  })
}, { immediate: true })

watch([marked, questions], schedule)

onBeforeUnmount(() => {
  cancelAnimationFrame(frame)
  publishMarks(owner, null)
  for (const place of places.values())
    place.remove()
  places.clear()
})
</script>

<template>
  <!-- What stands over the replies: the pictures' outlines. -->
  <div ref="layer" data-reply-comments class="pointer-events-none absolute inset-0 z-20">
    <div
      v-for="frame in outlines"
      :key="frame.key"
      :data-picture-outline="frame.kind"
      class="picture-outline absolute"
      :class="`picture-outline-${frame.kind}`"
      :style="{ top: `${frame.top}px`, left: `${frame.left}px`, width: `${frame.width}px`, height: `${frame.height}px` }"
    />
  </div>

  <!-- Each comment's box, under its line in the reply. -->
  <Teleport v-for="comment in marked" :key="comment.id" :to="placeOf(`comment:${comment.id}`)">
    <CommentItem
      class="my-2"
      inline
      :text="comment.text"
      :sent="comment.sent"
      @save="commenting.edit(comment.id, $event)"
      @delete="commenting.remove(comment.id)"
    />
  </Teleport>

  <!-- Each side question's card, under its line in the reply. -->
  <template v-if="asking">
    <Teleport v-for="question in questions" :key="question.id" :to="placeOf(`ask:${question.id}`)">
      <AskCard
        class="my-2"
        :folded="question.folded"
        :deletable="unanswered(question)"
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
        @stop="asking.stop(question.id)"
        @collapse="asking.fold(question.id, true)"
        @expand="asking.fold(question.id, false)"
        @delete="asking.remove(question.id)"
        @open-full="asking.openFull?.(question.id)"
      />
    </Teleport>
  </template>
</template>
