import { computed, nextTick, onScopeDispose, ref, type Ref, watch } from 'vue'
import { measureElement as measureRow, useVirtualizer } from '@tanstack/vue-virtual'
import { BOTTOM_THRESHOLD_PX, distanceFromBottom } from './scroll-bottom'
import { FOLD_MS } from '../ui/fold'

type ScrollIntent = 'up' | 'down' | null
interface VirtualizedBlock {
  id: string
  type: string
}

const OVERSCAN = 8
/** Space between transcript rows; a row placed after the list keeps it too. */
export const BLOCK_GAP = 4
const AUTO_SCROLL_REENGAGE_THRESHOLD = 1
/**
 * How long a click or key press in the list holds the view still: a row the
 * reader opens or folds changes its height over a fold, and what they acted
 * on stays where it is meanwhile.
 */
const READER_LAYOUT_MS = FOLD_MS + 150

const BLOCK_HEIGHT_ESTIMATES: Record<string, number> = {
  user: 40,
  steer: 40,
  agent_message: 28,
  pending_steer: 40,
  queue_divider: 36,
  queued_message: 40,
  resume: 0,
  thinking: 20,
  redacted_thinking: 0,
  text: 40,
  response: 24,
  tool_call: 28,
  error: 28,
  abort: 28,
  compaction_boundary: 36,
  compaction_marker: 36,
  compaction_progress: 36,
  history_edge: 36,
}

export interface ScrollAnchor {
  blockId: string
  anchorIndex: number
  offsetPx: number
  scrollTop: number
}

/** The row in view as the list follows it: where it starts in the list, which rows added or removed above it move. */
interface ViewAnchor extends ScrollAnchor {
  start: number
}

export interface PersistedScrollState {
  anchor: ScrollAnchor
  heightCache: Map<string, number>
}

export function useBlockVirtualizer(
  scrollContainer: Readonly<Ref<HTMLElement | undefined>>,
  blocks: Ref<VirtualizedBlock[]>,
  persistedState: PersistedScrollState | undefined,
) {
  /** The heights the list measured when it was last shown, until it measures them again. */
  const heightCache = new Map<string, number>(persistedState?.heightCache ?? [])

  const scrollOffset = ref(0)
  /** How far the list's end lies below the view, as of the last scroll or growth. */
  const endDistance = ref(0)
  const isRestored = ref(false)

  const virtualizer = useVirtualizer<HTMLElement, HTMLElement>(
    computed(() => ({
      count: blocks.value.length,
      // An empty list has no place yet: once it has rows, it starts at
      // `placement()`, so its first render is of the rows it will show.
      enabled: blocks.value.length > 0,
      initialOffset: placement,
      // Until the scroller is measured, as tall as the window, which it never
      // exceeds: the first render covers the view, at worst a little more.
      initialRect: { width: 0, height: window.innerHeight },
      getScrollElement: () => scrollContainer.value ?? null,
      estimateSize,
      // Vue calls a row's ref as it mounts the row, and a list that mounts
      // anew does so before it is in the document, where every row is 0 px
      // tall. Such a row keeps the height it is known by; the virtualizer
      // measures it once it watches the rows, when the scroller attaches.
      measureElement: (element, entry, instance) => element.isConnected
        ? measureRow(element, entry, instance)
        : estimateSize(instance.indexFromElement(element)),
      overscan: OVERSCAN,
      gap: BLOCK_GAP,
      getItemKey: (index: number) => {
        const block = blocks.value[index]
        return block ? block.id : index
      },
    })),
  )

  /** A row's height until it is measured: as last measured, or by its kind. */
  function estimateSize(index: number): number {
    const block = blocks.value[index]
    if (!block)
      return 40
    return heightCache.get(block.id) ?? (BLOCK_HEIGHT_ESTIMATES[block.type] ?? 40)
  }

  /**
   * Where the list starts when it first has rows, before any is measured:
   * where the reader left it, or its end. `restoreScroll` then puts the
   * scroller there and corrects for the rows' real heights; a list that
   * rendered at its top first would mount a screen of rows only to replace
   * them, which costs as much as the rows the reader sees.
   */
  function placement(): number {
    const anchor = persistedState?.anchor
    if (anchor && blocks.value.some((block) => block.id === anchor.blockId))
      return anchor.scrollTop
    return Math.max(0, virtualizer.value.getTotalSize() - window.innerHeight)
  }

  const virtualItems = computed(() => virtualizer.value.getVirtualItems())
  const totalSize = computed(() => virtualizer.value.getTotalSize())

  function measureElement(el: Element | null) {
    if (el instanceof HTMLElement)
      virtualizer.value.measureElement(el)
  }

  const shouldAutoScroll = ref(true)
  /**
   * The list follows its end, or its end is in view: otherwise the reader is
   * offered the way back to it. One rule for every way of leaving the end,
   * a small scroll up as much as a row opened at the end.
   */
  const isAtBottom = computed(() => shouldAutoScroll.value || endDistance.value <= AUTO_SCROLL_REENGAGE_THRESHOLD)
  let isProgrammaticScroll = false
  let pendingIntent: ScrollIntent = null
  let touchStartY = 0
  let furthestDistanceSinceDisengage = 0

  // The reader opened or folded something: until the fold ends, the view
  // neither follows the end nor corrects for the row's new height, so the row
  // they acted on stays put and what follows it moves, as a browser keeps a
  // <details> the reader opens in place. Then the list is at its end or not
  // by where it is: it follows only when its end is in view, as scrolling
  // there re-engages it. The nearness that keeps following while the reader
  // scrolls does not decide it, since a row opened at the end grows below the
  // view by less than that, and following would pull its steps up under the
  // reader the next time the list grows. A click that changed no row's
  // height, such as on a copy button, leaves following as it was, and the
  // view catches up with what arrived while it held.
  let readerLayoutUntil = 0
  let readerLayoutTimer: ReturnType<typeof setTimeout> | undefined
  /** The rows the reader acted on while the view holds, each with its height when first acted on. */
  const readerRows = new Map<HTMLElement, number>()
  function holdsView(): boolean {
    return performance.now() < readerLayoutUntil
  }
  function readerChangesLayout(event: Event): void {
    const row = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-index]') : null
    if (row && !readerRows.has(row))
      readerRows.set(row, row.getBoundingClientRect().height)
    readerLayoutUntil = performance.now() + READER_LAYOUT_MS
    clearTimeout(readerLayoutTimer)
    readerLayoutTimer = setTimeout(() => {
      readerLayoutTimer = undefined
      const el = scrollContainer.value
      const resized = [...readerRows].some(([row, height]) =>
        Math.abs(row.getBoundingClientRect().height - height) > AUTO_SCROLL_REENGAGE_THRESHOLD)
      readerRows.clear()
      if (!el)
        return
      const dist = distanceFromBottom(el)
      endDistance.value = dist
      if (resized || !shouldAutoScroll.value)
        shouldAutoScroll.value = dist <= AUTO_SCROLL_REENGAGE_THRESHOLD
      else
        scrollToBottom()
    }, READER_LAYOUT_MS)
  }
  onScopeDispose(() => {
    clearTimeout(readerLayoutTimer)
    readerRows.clear()
  })

  function scrollToBottom() {
    isProgrammaticScroll = true
    virtualizer.value.scrollToIndex(blocks.value.length - 1, { align: 'end' })
    isProgrammaticScroll = false
    furthestDistanceSinceDisengage = 0
  }

  /**
   * Scrolls the block `id` to the middle of the view and stops following the
   * end, as a search result opened at a message asks; false while the list
   * has no such block, such as before its history arrives.
   */
  function reveal(id: string): boolean {
    const index = blocks.value.findIndex((block) => block.id === id)
    if (index < 0)
      return false
    // The list is placed: the restore of a saved position no longer applies.
    isRestored.value = true
    shouldAutoScroll.value = false
    isProgrammaticScroll = true
    virtualizer.value.scrollToIndex(index, { align: 'center' })
    isProgrammaticScroll = false
    return true
  }

  function scrollToBottomExplicit() {
    shouldAutoScroll.value = true
    scrollToBottom()
  }

  function updateAutoScrollState(dist: number, threshold: number) {
    if (pendingIntent === 'up' || !shouldAutoScroll.value) {
      furthestDistanceSinceDisengage = Math.max(furthestDistanceSinceDisengage, dist)
    }
    if (isProgrammaticScroll)
      return
    if (pendingIntent === 'up') {
      shouldAutoScroll.value = false
    } else if (pendingIntent === 'down' &&
      dist <= threshold &&
      furthestDistanceSinceDisengage > threshold) {
      shouldAutoScroll.value = true
      furthestDistanceSinceDisengage = 0
    } else if (dist <= AUTO_SCROLL_REENGAGE_THRESHOLD) {
      shouldAutoScroll.value = true
      furthestDistanceSinceDisengage = 0
    } else if (dist > threshold) {
      shouldAutoScroll.value = false
    }
  }

  watch(
    scrollContainer,
    (el, _, onCleanup) => {
      if (!el)
        return
      const onWheel = (e: WheelEvent) => {
        if (e.deltaY < 0)
          pendingIntent = 'up'
        if (e.deltaY > 0)
          pendingIntent = 'down'
      }
      const onTouchStart = (e: TouchEvent) => {
        touchStartY = e.touches[0]?.clientY ?? 0
      }
      const onTouchMove = (e: TouchEvent) => {
        const touchY = e.touches[0]?.clientY ?? touchStartY
        if (touchY > touchStartY)
          pendingIntent = 'up'
        if (touchY < touchStartY)
          pendingIntent = 'down'
        touchStartY = touchY
      }
      const onKeydown = (e: KeyboardEvent) => {
        if (e.key === 'Enter' || e.key === ' ')
          readerChangesLayout(e)
      }
      el.addEventListener('click', readerChangesLayout, { capture: true })
      el.addEventListener('keydown', onKeydown, { capture: true })
      el.addEventListener('wheel', onWheel, { passive: true })
      el.addEventListener('touchstart', onTouchStart, { passive: true })
      el.addEventListener('touchmove', onTouchMove, { passive: true })
      onCleanup(() => {
        el.removeEventListener('click', readerChangesLayout, { capture: true })
        el.removeEventListener('keydown', onKeydown, { capture: true })
        el.removeEventListener('wheel', onWheel)
        el.removeEventListener('touchstart', onTouchStart)
        el.removeEventListener('touchmove', onTouchMove)
      })
    },
    { immediate: true },
  )

  watch(
    () => virtualizer.value.getTotalSize(),
    () => {
      if (!isRestored.value || !shouldAutoScroll.value || holdsView()) {
        // The list grew or shrank without following: its end moved off or
        // into the view with no scroll to say so.
        if (scrollContainer.value)
          endDistance.value = distanceFromBottom(scrollContainer.value)
        return
      }
      scrollToBottom()
    },
    { flush: 'post' },
  )

  watch(
    virtualizer,
    (instance) => {
      instance.shouldAdjustScrollPositionOnItemSizeChange = (
        item,
        _delta,
        currentInstance
      ) => {
        // Before the first measurement the virtualizer states no offset; the
        // top of the list is where it starts, which is what its own private
        // getter also falls back to.
        if (holdsView())
          return false
        const offset = currentInstance.scrollOffset ?? 0
        const isStreamingTail = item.index === blocks.value.length - 1
        const allowCorrection = shouldAutoScroll.value || !isStreamingTail
        return allowCorrection
          && item.start < offset + currentInstance.scrollAdjustments
      }
    },
    { immediate: true },
  )

  let lastAnchor: ViewAnchor | null = null

  function updateAnchor() {
    const el = scrollContainer.value
    if (!el || blocks.value.length === 0)
      return
    const st = el.scrollTop
    const containerTop = el.getBoundingClientRect().top
    for (const item of virtualizer.value.getVirtualItems()) {
      if (item.start + item.size > st) {
        const anchorEl = el.querySelector(`[data-index="${item.index}"]`) as HTMLElement | null
        if (!anchorEl)
          return
        lastAnchor = {
          blockId: blocks.value[item.index]!.id,
          anchorIndex: item.index,
          offsetPx: anchorEl.getBoundingClientRect().top - containerTop,
          scrollTop: st,
          start: item.start,
        }
        return
      }
    }
  }

  function onScroll() {
    const el = scrollContainer.value
    if (el) {
      scrollOffset.value = el.scrollTop
      const dist = distanceFromBottom(el)
      endDistance.value = dist
      updateAutoScrollState(dist, BOTTOM_THRESHOLD_PX)
      pendingIntent = null
    }
    updateAnchor()
  }

  function restoreScroll() {
    const el = scrollContainer.value
    if (!el || blocks.value.length === 0 || isRestored.value)
      return
    isRestored.value = true

    if (!persistedState) {
      scrollToBottom()
      nextTick(() => updateAnchor())
      return
    }

    const anchorIndex = blocks.value.findIndex((b) => b.id === persistedState.anchor.blockId)
    if (anchorIndex < 0) {
      scrollToBottom()
      nextTick(() => updateAnchor())
      return
    }

    el.scrollTop = persistedState.anchor.scrollTop
    correctUntilConverged(el, anchorIndex, persistedState.anchor.offsetPx, 3)
  }

  function correctUntilConverged(
    el: HTMLElement,
    anchorIndex: number,
    targetOffset: number,
    maxPasses: number
  ) {
    waitForScrollStable(el, () => {
      const anchorEl = el.querySelector(`[data-index="${anchorIndex}"]`) as HTMLElement | null
      if (!anchorEl) {
        updateAnchor()
        return
      }
      const correction =
        anchorEl.getBoundingClientRect().top -
        el.getBoundingClientRect().top -
        targetOffset
      if (Math.abs(correction) > 1 && maxPasses > 0) {
        el.scrollTop += correction
        correctUntilConverged(el, anchorIndex, targetOffset, maxPasses - 1)
      } else {
        updateAnchor()
      }
    })
  }

  watch(
    () => [blocks.value.length, scrollContainer.value] as const,
    ([len, el]) => {
      // A list that empties loses its place, and places itself again when
      // rows come, as the virtualizer then starts it at `placement()` again.
      if (len === 0)
        isRestored.value = false
      if (!isRestored.value && len > 0 && el) {
        nextTick(() => restoreScroll())
      }
    },
    { immediate: true, flush: 'post' },
  )

  // Rows added above the row in view, as an earlier page arrives, or
  // removed, as its loading row goes, move it down or up: the view moves by
  // as much, so the reader's row stays where it is, as a chat app keeps its
  // place when earlier messages load. A reader who follows the end follows
  // it instead.
  watch(
    () => blocks.value.map((block) => block.id),
    () => {
      const el = scrollContainer.value
      const anchor = lastAnchor
      if (!el || !anchor || !isRestored.value || shouldAutoScroll.value) {
        return
      }
      const index = blocks.value.findIndex((block) => block.id === anchor.blockId)
      if (index < 0 || index === anchor.anchorIndex) {
        return
      }
      const moved = virtualizer.value.measurementsCache[index]?.start
      if (moved === undefined) {
        return
      }
      // The virtualizer learns the scroller's offset from its scroll event,
      // which comes after the new rows are measured; corrected from the old
      // offset, they would pull the view back. So its offset moves with the
      // scroller's, in the same step.
      const target = (virtualizer.value.scrollOffset ?? el.scrollTop) + moved - anchor.start
      virtualizer.value.scrollOffset = target
      el.scrollTop = target
      updateAnchor()
    },
    { flush: 'post' },
  )

  function getPersistedState(): PersistedScrollState | undefined {
    if (!lastAnchor)
      return undefined
    return {
      anchor: {
        blockId: lastAnchor.blockId,
        anchorIndex: lastAnchor.anchorIndex,
        offsetPx: lastAnchor.offsetPx,
        scrollTop: scrollContainer.value?.scrollTop ?? lastAnchor.scrollTop
      },
      // The virtualizer's own measurements, keyed by block id, over the
      // heights it was given and has not measured again.
      heightCache: new Map([
        ...heightCache,
        ...[...virtualizer.value.itemSizeCache].filter((entry): entry is [string, number] => typeof entry[0] === 'string'),
      ]),
    }
  }

  return {
    virtualizer,
    virtualItems,
    totalSize,
    measureElement,
    scrollOffset,
    isAtBottom,
    scrollToBottom: scrollToBottomExplicit,
    reveal,
    onScroll,
    getPersistedState,
  }
}

function waitForScrollStable(el: HTMLElement, callback: () => void) {
  let prev = el.scrollTop
  let stable = 0
  function check() {
    if (el.scrollTop === prev) {
      if (++stable >= 2)
        return callback()
    } else {
      stable = 0
      prev = el.scrollTop
    }
    requestAnimationFrame(check)
  }
  requestAnimationFrame(check)
}
