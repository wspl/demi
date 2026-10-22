import { computed, nextTick, onScopeDispose, ref, shallowRef, type Ref, watch } from 'vue'
import { elementScroll, measureElement as measureRow, useVirtualizer, type Range, type VirtualItem, type Virtualizer } from '@tanstack/vue-virtual'
import { BOTTOM_THRESHOLD_PX, distanceFromBottom } from './scroll-bottom'
import { FOLD_MS } from '../ui/fold'
import { pageLog } from '../infra/log'
import { revealOffset } from '../ui/reveal'
import { moveRows, rowsBefore, rowTails, scrolledRows, type RowsBefore, type RowTail } from './row-motion'

type ScrollIntent = 'up' | 'down' | null
interface VirtualizedBlock {
  id: string
  type: string
}

/**
 * How many rows the list draws beyond each edge of the view, at most, so a
 * row is drawn before it scrolls in; `drawnRows` draws only those within a
 * view's height of the edge.
 */
const OVERSCAN = 8

/**
 * The rows the list draws: those in view, and beyond each edge up to
 * `range.overscan` rows that start or end within a view's height of it. A
 * transcript of long answers then draws no screens of them beyond the
 * next, which nobody sees as the list opens, while a run of short rows,
 * many to a screen, still has its rows ready.
 */
function drawnRows(range: Range, rows: readonly Pick<VirtualItem, 'start' | 'end'>[], view: { offset: number, height: number }): number[] {
  if (range.count === 0)
    return []
  let first = range.startIndex
  while (first > 0 && range.startIndex - first < range.overscan && rows[first - 1]!.end > view.offset - view.height)
    first -= 1
  let last = range.endIndex
  while (last < range.count - 1 && last - range.endIndex < range.overscan && rows[last + 1]!.start < view.offset + 2 * view.height)
    last += 1
  return Array.from({ length: last - first + 1 }, (_, index) => first + index)
}

/**
 * The rows that lie in the view, by first and last index; the view's first row
 * when none does.
 */
function rowsInView(rows: readonly Pick<VirtualItem, 'start' | 'end'>[], view: { offset: number, height: number }): Pick<Range, 'startIndex' | 'endIndex'> {
  const firstAfter = rows.findIndex((row) => row.end > view.offset)
  const startIndex = Math.max(0, firstAfter)
  let endIndex = startIndex
  while (endIndex < rows.length - 1 && rows[endIndex + 1]!.start < view.offset + view.height)
    endIndex += 1
  return { startIndex, endIndex }
}

/** Space between transcript rows; a row placed after the list keeps it too. */
export const BLOCK_GAP = 4
/**
 * The reader is at the end while the end of the list is in view or less than
 * this far below it: the list follows its end from there.
 */
const AT_END_PX = 16
/** A row the reader acted on changed its height by more than rounding. */
const RESIZED_PX = 1
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
  pending_call: 28,
  // A run's row folded is its 28px face, as the tail row it takes over from is.
  work_group: 28,
  activity_slot: 28,
  error: 28,
  abort: 28,
  compaction_boundary: 36,
  compaction_marker: 36,
  compaction_progress: 36,
  history_edge: 36,
  delivery_failure: 64,
  session_failure: 52,
}

export interface ScrollAnchor {
  blockId: string
  anchorIndex: number
  offsetPx: number
  scrollTop: number
}

/**
 * The row in view as the list follows it: where it starts in the list, which
 * rows added or removed above it move.
 */
interface ViewAnchor extends ScrollAnchor {
  start: number
}

export interface PersistedScrollState {
  anchor: ScrollAnchor
  heightCache: Map<string, number>
}

/**
 * How many `row.missing` lines one list writes, since a list that drew a row it
 * does not hold draws it again in every frame.
 */
const MISSING_LINES = 5
/** How long the scroll stays still before the list writes where it stopped. */
const SCROLL_SETTLE_MS = 250

export interface BlockVirtualizerOptions<Block extends VirtualizedBlock> {
  /** The conversation and agent the list shows, which its lines are written under. */
  log?: { conversation: () => string | undefined, node: () => string | null | undefined }
  /**
   * What names a row across the forms its block takes; the block's id by
   * default. A row named the same is kept, with its measured height, rather
   * than built anew and laid out at a guess for a frame.
   */
  keyOf?: (block: Block) => string
}

export function useBlockVirtualizer<Block extends VirtualizedBlock>(
  scrollContainer: Readonly<Ref<HTMLElement | undefined>>,
  blocks: Ref<Block[]>,
  persistedState: PersistedScrollState | undefined,
  options: BlockVirtualizerOptions<Block> = {},
) {
  const keyOf = options.keyOf ?? ((block: Block) => block.id)
  /** Writes a line of the list under its conversation, with its agent and size. */
  function logList(level: 'debug' | 'info' | 'error', event: string, fields: Record<string, unknown> = {}): void {
    const node = options.log?.node()
    pageLog[level](event, { ...(node ? { node } : {}), rows: blocks.value.length, ...fields }, options.log?.conversation())
  }
  /** The rows in view, as the first and last index the list's own measurements put there. */
  function viewRows(): { first: number, last: number } | null {
    const el = scrollContainer.value
    const rows = virtualizer.value.measurementsCache
    if (!el || rows.length === 0)
      return null
    const { startIndex, endIndex } = rowsInView(rows, { offset: el.scrollTop, height: el.clientHeight })
    return { first: startIndex, last: endIndex }
  }
  /** What the virtualizer moved the view by to keep the row in view, since the last line of the scroll. */
  let resizeCorrections = 0
  let resizeCorrectedPx = 0
  /** Lines written of rows the list drew and does not hold, by their count (`MISSING_LINES`). */
  let missingLines = 0
  /**
   * The heights the list measured when it was last shown, by row key, until it
   * measures them again.
   */
  const heightCache = new Map<string, number>(persistedState?.heightCache ?? [])

  const scrollOffset = ref(0)
  /**
   * Where the list last scrolled itself to, by the virtualizer or here: a scroll
   * event that stands elsewhere is the reader's.
   */
  let scrolledTo = Number.NaN
  /**
   * How far the list's end lies below the view, as of the last scroll or
   * growth.
   */
  const endDistance = ref(0)
  /**
   * How far below the view's top the last block brought into view starts
   * (`revealOffset`): the virtualizer's room above a row it scrolls to the
   * start, which only `reveal` asks for.
   */
  let revealRoom = 0
  /**
   * The row `reveal` placed at a guess of its height, until the list
   * measures it; or the row a part of which it placed, such as a step of a
   * run that opens to show it, placed again as each measurement of the row
   * comes until `until`, while the row grows over its fold.
   */
  let revealing: { key: string, coveredBottom: number, part: { selector: string, until: number } | null } | null = null
  const isRestored = ref(false)

  /** The virtualizer exists: what reads it may. */
  const created = shallowRef(false)
  /** Where the view is and how tall, as the scroller says it now. */
  function view(instance: Virtualizer<HTMLElement, HTMLElement>): { offset: number, height: number } {
    return {
      offset: scrollContainer.value?.scrollTop ?? instance.scrollOffset ?? 0,
      height: instance.scrollRect?.height ?? window.innerHeight,
    }
  }
  /**
   * The rows `drawnRows` draws for where the view is, as a key. The virtualizer
   * works its rows out again only when its own range of rows in view, its count
   * or its options change, never as the view moves within a row taller than it,
   * where the rows drawn beyond the view do change. So the range extractor is
   * made anew whenever this does. It follows the scroll offset the list reads
   * and each change the virtualizer announces, its measurements and the view's
   * size among them.
   */
  const drawn = computed((): string => {
    void scrollOffset.value
    if (!created.value)
      return ''
    const instance: Virtualizer<HTMLElement, HTMLElement> = virtualizer.value
    const rows = instance.measurementsCache
    const at = view(instance)
    return drawnRows({ ...rowsInView(rows, at), overscan: OVERSCAN, count: rows.length }, rows, at).join()
  })

  /**
   * A range extractor of its own for the rows `key` names, so the virtualizer
   * draws them (`drawn`).
   */
  function extractorFor(key: string): (range: Range) => number[] {
    return (range) => {
      void key
      const instance: Virtualizer<HTMLElement, HTMLElement> = virtualizer.value
      return drawnRows(range, instance.measurementsCache, view(instance))
    }
  }

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
      scrollToFn: (offset, options, instance) => {
        // The virtualizer scrolls to follow a row's growth or shrinking inside
        // the observer's callback, before the list is updated: the rows are
        // read before the scroll moves them.
        seeRows()
        elementScroll(offset, options, instance)
        scrolledTo = scrollContainer.value?.scrollTop ?? Number.NaN
      },
      estimateSize,
      // A row is measured only in the document (`measureElement` below), as
      // the observer reports only rows there.
      measureElement: (element, entry, instance) => {
        const size = measureRow(element, entry, instance)
        const index = instance.indexFromElement(element)
        const key = instance.options.getItemKey(index)
        const known = instance.itemSizeCache.get(key)
        if (known !== undefined && size < known)
          keepReaderControl(element)
        // The virtualizer records a measurement only when it differs from the
        // estimate. A row measured as estimated is a size too: kept by its key,
        // it stays when the row's kind changes under the key, instead of
        // falling back to the estimate of the new kind, which is another size.
        if (known === undefined && size === estimateSize(index))
          instance.itemSizeCache.set(key, size)
        revealMeasured(index, size)
        return size
      },
      scrollPaddingStart: revealRoom,
      overscan: OVERSCAN,
      rangeExtractor: extractorFor(drawn.value),
      gap: BLOCK_GAP,
      getItemKey: (index: number) => {
        const block = blocks.value[index]
        return block ? keyOf(block) : index
      },
    })),
  )
  created.value = true

  /** A row's height until it is measured: as last measured, or by its kind. */
  function estimateSize(index: number): number {
    const block = blocks.value[index]
    if (!block)
      return 40
    return heightCache.get(keyOf(block)) ?? (BLOCK_HEIGHT_ESTIMATES[block.type] ?? 40)
  }

  /**
   * Where the list starts when it first has rows, before any is measured: where
   * the reader left it, or its end. `restoreScroll` then puts the scroller
   * there and corrects for the rows' real heights. A list that rendered at its
   * top first would mount a screen of rows only to replace them, which costs as
   * much as the rows the reader sees.
   */
  function placement(): number {
    const anchor = persistedState?.anchor
    if (anchor && blocks.value.some((block) => block.id === anchor.blockId))
      return anchor.scrollTop
    return Math.max(0, virtualizer.value.getTotalSize() - window.innerHeight)
  }

  const virtualItems = computed(() => {
    const items = virtualizer.value.getVirtualItems()
    // A row the virtualizer draws that the list no longer holds is a defect of
    // the page, which a render then fails on: the window says what it was.
    const missing = items.find((item) => item.index >= blocks.value.length)
    if (missing && missingLines < MISSING_LINES) {
      missingLines += 1
      logList('error', 'row.missing', {
        index: missing.index,
        window_first: items[0]?.index ?? null,
        window_last: items.at(-1)?.index ?? null,
        lines_written: missingLines,
      })
    }
    return items
  })
  const totalSize = computed(() => virtualizer.value.getTotalSize())

  /**
   * The row elements the virtualizer watches, whose later changes of size its
   * resize observer reports.
   */
  const watched = new WeakSet<HTMLElement>()

  /**
   * Hands the virtualizer a row as it mounts, which it measures once the
   * update that mounts it has run, and then watches. Vue calls a row's ref
   * while it mounts the row, before the row's own components have run their
   * mounted hooks, where a long message folds: measured then, the row would
   * be known by a height it never shows, which a row that leaves the list
   * before the observer reports again keeps. The measure still comes before
   * the frame is painted, so the row is never drawn at a guess.
   *
   * Vue calls a row's ref again at every update of the list, as a turn
   * streams; measuring each row again there would read its height while the
   * styles the update changed are not yet computed, so the browser would
   * recompute them for each row. A row's later sizes come from the observer,
   * after the browser laid the frame out.
   */
  function measureElement(el: Element | null) {
    if (!(el instanceof HTMLElement) || watched.has(el))
      return
    watched.add(el)
    void nextTick(() => {
      // By then the list that mounted the row is in the document, where the
      // row has its height; a row the same update mounted and removed again
      // was never shown.
      if (el.isConnected)
        virtualizer.value.measureElement(el)
    })
  }

  const shouldAutoScroll = ref(true)
  /** Whether the list follows its end, as `shouldAutoScroll` says; every change is a line, with its cause. */
  function follow(following: boolean, cause: string): void {
    if (shouldAutoScroll.value === following)
      return
    shouldAutoScroll.value = following
    logList('info', 'list.follow_changed', { following, cause, end_distance_px: Math.round(endDistance.value) })
  }
  /**
   * The list follows its end, or its end is in view: otherwise the reader is
   * offered the way back to it. One rule for every way of leaving the end,
   * a small scroll up as much as a row opened at the end.
   */
  const isAtBottom = computed(() => shouldAutoScroll.value || endDistance.value < AT_END_PX)
  let isProgrammaticScroll = false
  let pendingIntent: ScrollIntent = null
  let touchStartY = 0
  let furthestDistanceSinceDisengage = 0

  // The reader opened or folded something. Until the fold ends, the view
  // neither follows the end nor corrects for the row's new height, so the row
  // they acted on stays put and what follows it moves, as a browser keeps a
  // <details> the reader opens in place. A row that gets shorter, such as a
  // long message folded from Show Less at its end, keeps the control the
  // reader used where it was instead, its Show More where the Show Less
  // stood, as chat apps keep a message folded back in view: kept by its top,
  // the view would land on whatever follows the row, or at the list's end.
  //
  // Then the list is at its end or not by where it is: it follows only when its
  // end is in view, as scrolling there re-engages it. The nearness that keeps
  // following while the reader scrolls does not decide it, since a row opened
  // at the end grows below the view by less than that, and following would pull
  // its steps up under the reader the next time the list grows.
  //
  // A click that changed no row's height, such as on a copy button, leaves
  // following as it was, and the view catches up with what arrived while it
  // held.
  let readerLayoutUntil = 0
  let readerLayoutTimer: ReturnType<typeof setTimeout> | undefined
  /**
   * The rows the reader acted on while the view holds, each with its height
   * when first acted on.
   */
  const readerRows = new Map<HTMLElement, number>()
  /**
   * The control the reader last acted on while the view holds, and how far
   * below the view's top it stood then.
   */
  let readerControl: { element: Element, top: number } | null = null
  function holdsView(): boolean {
    return performance.now() < readerLayoutUntil
  }
  function readerChangesLayout(event: Event): void {
    const target = event.target instanceof Element ? event.target : null
    const row = target?.closest<HTMLElement>('[data-index]') ?? null
    if (row && !readerRows.has(row))
      readerRows.set(row, row.getBoundingClientRect().height)
    const control = target?.closest('button, summary, a, [role="button"]') ?? target
    readerControl = control ? { element: control, top: control.getBoundingClientRect().top } : null
    readerLayoutUntil = performance.now() + READER_LAYOUT_MS
    clearTimeout(readerLayoutTimer)
    readerLayoutTimer = setTimeout(() => {
      readerLayoutTimer = undefined
      const el = scrollContainer.value
      const resized = [...readerRows].some(([row, height]) =>
        Math.abs(row.getBoundingClientRect().height - height) > RESIZED_PX)
      readerRows.clear()
      readerControl = null
      if (!el)
        return
      const dist = distanceFromBottom(el)
      endDistance.value = dist
      if (resized || !shouldAutoScroll.value)
        follow(dist < AT_END_PX, 'reader_changed_layout')
      else
        scrollToBottom()
    }, READER_LAYOUT_MS)
  }
  onScopeDispose(() => {
    clearTimeout(readerLayoutTimer)
    readerRows.clear()
    readerControl = null
  })

  /**
   * While the view holds, scrolls it by as much as the control the reader
   * acted on in `row` moved, so it stands where it stood. It runs as the
   * shorter row is measured, before the list lays the rows out by its new
   * height: a row above the view by then would leave the list, and the
   * control with it.
   */
  function keepReaderControl(row: HTMLElement): void {
    const el = scrollContainer.value
    const control = readerControl
    if (!el || !holdsView() || !control || !row.contains(control.element))
      return
    const moved = control.element.getBoundingClientRect().top - control.top
    if (Math.abs(moved) <= RESIZED_PX)
      return
    // The virtualizer's offset moves with the scroller's in the same step, as
    // it does when rows are added above the view.
    const target = el.scrollTop + moved
    virtualizer.value.scrollOffset = target
    el.scrollTop = target
    scrolledTo = el.scrollTop
    scrollOffset.value = el.scrollTop
    logList('debug', 'list.anchor_corrected', { reason: 'reader_control', delta_px: Math.round(moved) })
  }

  function scrollToBottom() {
    isProgrammaticScroll = true
    virtualizer.value.scrollToIndex(blocks.value.length - 1, { align: 'end' })
    isProgrammaticScroll = false
    furthestDistanceSinceDisengage = 0
  }

  /**
   * Brings the block `id` into view where `revealOffset` places it, in the part
   * of the view `coveredBottom` pixels of the dock leave visible, and stops
   * following the end, as a search result opened at a message asks. False while
   * the list has no such block, such as before its history arrives.
   *
   * A row not measured yet is placed by its estimated height, and placed again
   * by its real one once it is measured. With `part`, a selector of an element
   * inside the row, such as a step of an open run, that element is what
   * `revealOffset` places, once the row draws it.
   */
  function reveal(id: string, coveredBottom = 0, part?: string): boolean {
    const index = blocks.value.findIndex((block) => block.id === id)
    if (index < 0) {
      logList('info', 'list.revealed', { block: id, row: -1, part: part !== undefined })
      return false
    }
    // The list is placed: the restore of a saved position no longer applies.
    isRestored.value = true
    follow(false, 'reveal')
    logList('info', 'list.revealed', { block: id, row: index, part: part !== undefined })
    const key = keyOf(blocks.value[index]!)
    if (part !== undefined)
      revealing = { key, coveredBottom, part: { selector: part, until: performance.now() + READER_LAYOUT_MS } }
    else
      revealing = virtualizer.value.itemSizeCache.has(key) ? null : { key, coveredBottom, part: null }
    placeRevealed(index, virtualizer.value.measurementsCache[index]?.size ?? estimateSize(index), coveredBottom, part)
    return true
  }

  /** A reveal places a part of a row whose fold may still be opening. */
  function placingPart(): boolean {
    return !!revealing?.part && performance.now() <= revealing.part.until
  }

  /**
   * Where the element `part` names stands inside the row at `index` as drawn;
   * null while the row does not draw it.
   */
  function partBox(index: number, part: string): { top: number, height: number } | null {
    const row = scrollContainer.value?.querySelector(`[data-index="${index}"]`)
    const found = row?.querySelector(part)
    if (!row || !found)
      return null
    const box = found.getBoundingClientRect()
    return { top: box.top - row.getBoundingClientRect().top, height: box.height }
  }

  function placeRevealed(index: number, size: number, coveredBottom: number, part?: string): void {
    const el = scrollContainer.value
    if (!el)
      return
    const view = el.clientHeight - coveredBottom
    const box = part === undefined ? null : partBox(index, part)
    // A part stands where the rule places it, its row starting above it by as
    // much.
    revealRoom = box ? revealOffset(box.height, view) - box.top : revealOffset(size, view)
    // The room applies at once, to this scroll and to the virtualizer's
    // corrections of it while the rows around it are measured.
    virtualizer.value.setOptions({ ...virtualizer.value.options, scrollPaddingStart: revealRoom })
    isProgrammaticScroll = true
    virtualizer.value.scrollToIndex(index, { align: 'start' })
    isProgrammaticScroll = false
  }

  /**
   * The row `reveal` placed at a guess measured `size`: placed again when
   * its real height places it elsewhere. A row whose part it placed is
   * placed again at each measurement until its fold has had time to end,
   * as the list's room below grows with it.
   */
  function revealMeasured(index: number, size: number): void {
    const block = blocks.value[index]
    if (!revealing || !block || keyOf(block) !== revealing.key)
      return
    const { coveredBottom, part } = revealing
    if (part) {
      if (performance.now() > part.until) {
        revealing = null
        return
      }
      // After the measure returns, as below.
      queueMicrotask(() => placeRevealed(index, size, coveredBottom, part.selector))
      return
    }
    revealing = null
    const el = scrollContainer.value
    if (el && revealOffset(size, el.clientHeight - coveredBottom) !== revealRoom)
      // After the measure returns, so the list holds the height it places by.
      queueMicrotask(() => placeRevealed(index, size, coveredBottom))
  }

  function scrollToBottomExplicit() {
    follow(true, 'jump_to_end')
    scrollToBottom()
  }

  function updateAutoScrollState(dist: number, threshold: number) {
    if (pendingIntent === 'up' || !shouldAutoScroll.value) {
      furthestDistanceSinceDisengage = Math.max(furthestDistanceSinceDisengage, dist)
    }
    // A part placed while its row opens can land at the list's end before
    // the row has grown below it: the reveal ended following, and the list
    // does not take that end for the reader's.
    if (isProgrammaticScroll || placingPart())
      return
    if (pendingIntent === 'up') {
      follow(false, 'reader_scrolled_up')
    } else if (pendingIntent === 'down' &&
      dist <= threshold &&
      furthestDistanceSinceDisengage > threshold) {
      follow(true, 'reader_scrolled_to_end')
      furthestDistanceSinceDisengage = 0
    } else if (dist < AT_END_PX) {
      follow(true, 'end_in_view')
      furthestDistanceSinceDisengage = 0
    } else if (dist > threshold) {
      follow(false, 'left_end')
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

  /**
   * The list grew or shrank at its end: a reader who follows it sees its end
   * in the frame that draws the change. Rows the virtualizer measures change
   * its total size, which it reports in the same frame; what follows the
   * rows reports its own size, as a resize observer does after layout and
   * before paint.
   */
  function followEnd(): void {
    if (!isRestored.value || !shouldAutoScroll.value || holdsView()) {
      // The list grew or shrank without following: its end moved off or
      // into the view with no scroll to say so.
      if (scrollContainer.value)
        endDistance.value = distanceFromBottom(scrollContainer.value)
      return
    }
    scrollToBottom()
  }
  watch(() => virtualizer.value.getTotalSize(), followEnd, { flush: 'post' })

  // Content that comes or goes moves the rows to their places over
  // `ROW_MOVE_MS`, not in one frame (`moveRows`). They move from where the
  // reader saw them: the rows are read before the first update since the last
  // frame was painted, and later updates before the next paint, such as the
  // measure of rows that just mounted, move them from there too, so a place
  // the reader never saw is never the start of a move. What the reader scrolls
  // in between moves no row (`onScroll`), nor does a row they opened or a list
  // not yet placed.
  let seen: RowsBefore | null = null
  let unpainted = false
  // How tall the list placed each row, by key, when it last settled.
  const placedHeights = () => new Map(virtualizer.value.getVirtualItems().map((item) => [String(item.key), item.size]))
  let settled = new Map<string, number>()
  // Where each row's text ended in it then, for a row that grew above its end.
  let settledTails = new Map<string, RowTail>()
  const layout = () => `${virtualizer.value.getTotalSize()}|${virtualizer.value.getVirtualItems().map((item) => `${String(item.key)}:${item.start}`).join()}`
  function seeRows(): void {
    const el = scrollContainer.value
    if (unpainted) {
      if (!el || !isRestored.value || holdsView())
        seen = null
      return
    }
    seen = el && isRestored.value && !holdsView() ? rowsBefore(el, settled, settledTails) : null
    unpainted = true
    // The frame that follows is painted at the end of the second callback.
    requestAnimationFrame(() => requestAnimationFrame(() => {
      unpainted = false
    }))
  }
  watch(layout, seeRows, { flush: 'pre' })
  watch(layout, () => {
    const el = scrollContainer.value
    if (el && seen)
      moveRows(el, seen, placedHeights(), shouldAutoScroll.value)
    settled = placedHeights()
    settledTails = scrollContainer.value ? rowTails(scrollContainer.value) : new Map()
  }, { flush: 'post' })

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
        // The row whose part a reveal placed grows below that part, as its
        // fold opens: the reveal places the part again instead.
        if (placingPart() && item.key === revealing?.key)
          return false
        const offset = currentInstance.scrollOffset ?? 0
        const isStreamingTail = item.index === blocks.value.length - 1
        const allowCorrection = shouldAutoScroll.value || !isStreamingTail
        const adjusts = allowCorrection
          && item.start < offset + currentInstance.scrollAdjustments
        if (adjusts) {
          // Many a second as rows are measured: counted, and told with the scroll that follows.
          resizeCorrections += 1
          resizeCorrectedPx += Math.round(_delta)
        }
        return adjusts
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
      // The loading row at an edge is no content: a page that arrives above
      // it moves what the reader reads, and an anchor on the row would leave
      // the view at the top of the list, on the row, where the next page is
      // asked for again.
      if (blocks.value[item.index]?.type === 'history_edge')
        continue
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

  /**
   * The first and last row that lie within `reach` px of the view, above its
   * top and below its bottom; none while the list has no row. Rows are read
   * from the virtualizer's measurements, drawn or not.
   */
  function rowsNear(reach: number): { first: number, last: number } | null {
    const el = scrollContainer.value
    const rows = virtualizer.value.measurementsCache
    if (!el || rows.length === 0)
      return null
    const top = el.scrollTop
    const bottom = top + el.clientHeight
    const first = rows.findIndex((row) => row.end > top - reach)
    const last = rows.findLastIndex((row) => row.start < bottom + reach)
    return first < 0 || last < 0 ? null : { first, last }
  }

  function onScroll() {
    const el = scrollContainer.value
    if (el) {
      scrollOffset.value = el.scrollTop
      // What the reader scrolled is no move of any row: a place kept from before
      // it stands further along.
      if (seen && Math.abs(el.scrollTop - scrolledTo) >= 1)
        seen = scrolledRows(seen, el.scrollTop)
      const dist = distanceFromBottom(el)
      endDistance.value = dist
      updateAutoScrollState(dist, BOTTOM_THRESHOLD_PX)
      pendingIntent = null
      if (scrollSettle === undefined)
        scrolledFrom = scrollOffsetAtStart(el)
      clearTimeout(scrollSettle)
      scrollSettle = setTimeout(() => {
        scrollSettle = undefined
        writeScrolled()
      }, SCROLL_SETTLE_MS)
    }
    updateAnchor()
  }

  /** Where the current scroll began, and the timer that ends it once the view stays still. */
  let scrolledFrom = 0
  let scrollSettle: ReturnType<typeof setTimeout> | undefined
  function scrollOffsetAtStart(el: HTMLElement): number {
    return lastScrollTop ?? el.scrollTop
  }
  /** The scroll position the list last wrote a line at, or knew at the last settle. */
  let lastScrollTop: number | undefined
  /** Writes where a scroll stopped: from and to, the rows in view, and what the list corrected by itself meanwhile. */
  function writeScrolled(): void {
    const el = scrollContainer.value
    if (!el)
      return
    const to = Math.round(el.scrollTop)
    const from = Math.round(scrolledFrom)
    lastScrollTop = el.scrollTop
    const inView = viewRows()
    if (from !== to || resizeCorrections > 0) {
      logList('debug', 'list.scrolled', {
        from_px: from,
        to_px: to,
        first: inView?.first ?? null,
        last: inView?.last ?? null,
        following: shouldAutoScroll.value,
        resize_corrections: resizeCorrections,
        resize_corrected_px: resizeCorrectedPx,
        total_px: Math.round(virtualizer.value.getTotalSize()),
      })
    }
    resizeCorrections = 0
    resizeCorrectedPx = 0
  }
  onScopeDispose(() => clearTimeout(scrollSettle))

  function restoreScroll() {
    const el = scrollContainer.value
    if (!el || blocks.value.length === 0 || isRestored.value)
      return
    isRestored.value = true

    if (!persistedState) {
      scrollToBottom()
      scrollOffset.value = el.scrollTop
      lastScrollTop = el.scrollTop
      logList('info', 'list.placed', { how: 'end', saved: false, scroll_top_px: Math.round(el.scrollTop) })
      nextTick(() => updateAnchor())
      return
    }

    const anchorIndex = blocks.value.findIndex((b) => b.id === persistedState.anchor.blockId)
    if (anchorIndex < 0) {
      scrollToBottom()
      scrollOffset.value = el.scrollTop
      lastScrollTop = el.scrollTop
      logList('info', 'list.placed', { how: 'end', saved: true, saved_row_held: false, scroll_top_px: Math.round(el.scrollTop) })
      nextTick(() => updateAnchor())
      return
    }

    el.scrollTop = persistedState.anchor.scrollTop
    lastScrollTop = el.scrollTop
    logList('info', 'list.placed', { how: 'saved_place', saved: true, saved_row_held: true, row: anchorIndex, scroll_top_px: Math.round(el.scrollTop) })
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
        logList('debug', 'list.anchor_corrected', { reason: 'restore', delta_px: Math.round(correction), passes_left: maxPasses - 1 })
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
      scrolledTo = el.scrollTop
      logList('debug', 'list.anchor_corrected', {
        reason: moved > anchor.start ? 'rows_added_above' : 'rows_removed_above',
        delta_px: Math.round(moved - anchor.start),
        row_from: anchor.anchorIndex,
        row_to: index,
      })
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
    /** Whether the list placed itself, at the end or where it was left or a reveal put it: before, `scrollOffset` is its first layout's, not the reader's. */
    placed: isRestored,
    isAtBottom,
    scrollToBottom: scrollToBottomExplicit,
    reveal,
    rowsNear,
    onScroll,
    getPersistedState,
    /**
     * The id of the row at the top of the view now, none before the list has
     * a row in view. It is read from the rows as drawn, not from the last
     * scroll: rows that changed under a view that did not scroll, as a
     * folded run does when a page arrives above it, are not the ones the last
     * scroll saw.
     */
    anchorId: (): string | undefined => {
      updateAnchor()
      return lastAnchor?.blockId
    },
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
