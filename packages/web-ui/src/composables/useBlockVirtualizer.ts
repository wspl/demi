import { keptComputed } from '../infra/keep-same'
import { computed, nextTick, onScopeDispose, readonly, ref, shallowRef, type Ref, watch } from 'vue'
import { elementScroll, measureElement as measureRow, observeElementOffset, useVirtualizer, type Range, type VirtualItem, type Virtualizer } from '@tanstack/vue-virtual'
import { BOTTOM_THRESHOLD_PX, distanceFromBottom } from './scroll-bottom'
import { FOLD_MS } from '../ui/fold'
import { pageLog } from '../infra/log'
import { deviceSystem } from '../auth/device-unlock'
import { revealOffset } from '../ui/reveal'
import { prefersReducedMotion } from '../ui/stream-reveal'
import { moveRows, rowsBefore, rowTails, scrolledRows, stopRowMoves, tailDisplaced, type RowsBefore, type RowTail } from './row-motion'

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

/** Whether a control or a <details> is open: its `aria-expanded`, or its `open`. */
function openState(element: Element): string | null {
  return element instanceof HTMLDetailsElement ? String(element.open) : element.getAttribute('aria-expanded')
}

/** What a row's controls and <details> hold open, each by its element. */
function disclosures(row: HTMLElement): Map<Element, string | null> {
  return new Map([...row.querySelectorAll('[aria-expanded], details')].map((element) => [element, openState(element)]))
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
  opaque: 28,
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
  /**
   * The row the reader left in view, where the list opens again; null when
   * the reader followed the end, where it opens again with what arrived
   * meanwhile, as a chat app reopens a conversation left at its latest.
   */
  anchor: ScrollAnchor | null
  heightCache: Map<string, number>
}

/**
 * How many `row.missing` lines one list writes, since a list that drew a row it
 * does not hold draws it again in every frame.
 */
const MISSING_LINES = 5
/**
 * How quickly the view glides to the end it follows: the distance left
 * shrinks by e in this many milliseconds, so most of a new line is scrolled
 * in within about 150 ms, as long as text streams, without a step.
 */
const GLIDE_MS = 50
/** The keys that scroll a focused list. */
const SCROLL_KEYS = new Set(['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End', ' '])
/** How long the scroll stays still before the list writes where it stopped. */
const SCROLL_SETTLE_MS = 250
/**
 * How long after its last scroll event, or the finger's lifting, a scroll
 * has ended, momentum included, which goes on sending scroll events while it
 * runs. Longer than the 150 ms the virtualizer holds its own corrections back
 * on iOS after either, so none it holds back is left to it.
 */
const MOMENTUM_END_MS = 200

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
    const { startIndex, endIndex } = rowsInView(rows, { offset: viewTop(el), height: el.clientHeight })
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

  /**
   * Rows above the view that change height move the rows in view, and the
   * view moves by as much to keep them still. The list makes each such
   * correction itself, never the virtualizer, which scrolls as the row is
   * measured: the list's box has not grown yet then, so near the end the
   * browser clamps the position, and the rows in view are pushed down by
   * what it cut off. The list scrolls once the update that lays the rows out
   * by their heights has run, before the frame is painted.
   *
   * iOS stops a scroll's momentum when a page writes the scroll position, so
   * while the reader scrolls there the rows are drawn `rowsShift` px above
   * their places instead, the view's own position unwritten; once the scroll
   * ends, the view takes the shift over in one step that moves nothing on
   * screen. Virtuoso keeps a list still on iOS the same way.
   */
  const shiftsRows = typeof navigator !== 'undefined'
    && deviceSystem({ userAgent: navigator.userAgent, maxTouchPoints: navigator.maxTouchPoints }) === 'ios'
  /** How far above their places the rows are drawn, with the list's box. */
  const rowsShift = ref(0)
  /**
   * Where the view starts in the list while a settled shift is written: the
   * update that draws the rows at their places runs before the scroller
   * takes the shift over.
   */
  let settlingTop: number | null = null
  /**
   * What the rows above the view moved the rows in view by, unless the rows
   * are shifted for it: the view scrolls by as much once the update that
   * lays the rows out by their heights has run, when the list is tall enough
   * for it.
   */
  let pendingScrollBy = 0
  /**
   * Where the scroller stood when the first of those moves came: the view
   * scrolls from there, as the update that lays the rows out can clamp the
   * position when the list gets shorter.
   */
  let pendingFrom: number | null = null
  /** A finger is on the list. */
  let touching = false
  /** Ends the reader's scroll `MOMENTUM_END_MS` after its last scroll event. */
  let momentumEnd: ReturnType<typeof setTimeout> | undefined
  /** Where the view starts in the list: the scroller's position, and the rows' shift below it. */
  function viewTop(el: HTMLElement): number {
    return settlingTop ?? el.scrollTop + rowsShift.value
  }
  /** The reader scrolls the list now: a finger is on it, or its scroll has not ended. */
  function readerScrolls(): boolean {
    return touching || momentumEnd !== undefined
  }
  /**
   * The rows above the view moved the rows in view by `by` px: the view
   * follows them, by the rows' shift while the reader scrolls on iOS, by a
   * scroll after the update otherwise. The virtualizer's place for the view
   * moves at once, so the update draws the rows the view will show.
   */
  function followRowsAbove(by: number): void {
    virtualizer.value.scrollOffset = (virtualizer.value.scrollOffset ?? 0) + by
    scrollOffset.value = virtualizer.value.scrollOffset
    if (shiftsRows && readerScrolls()) {
      rowsShift.value += by
      return
    }
    pendingFrom ??= scrollContainer.value?.scrollTop ?? null
    pendingScrollBy += by
  }
  /** Scrolls the view by what `followRowsAbove` held for the update that has now run. */
  function scrollPending(): void {
    const el = scrollContainer.value
    const by = pendingScrollBy
    const from = pendingFrom
    pendingScrollBy = 0
    pendingFrom = null
    if (!el || from === null || by === 0)
      return
    el.scrollTop = from + by
    scrolledTo = el.scrollTop
  }
  /**
   * The view takes the rows' shift over: the rows are drawn at their places
   * and, once that update has run, the scroller moves by as much, which moves
   * nothing on screen. It waits while the reader scrolls, and while iOS
   * bounces the list past an edge, where a written position ends the bounce
   * early, unless `now` asks for it, as when the list scrolls itself or the
   * view reaches an edge the shift keeps it from passing.
   */
  function settleShift(now = false): Promise<void> | undefined {
    const el = scrollContainer.value
    if (!el || rowsShift.value === 0)
      return undefined
    if (!now && (readerScrolls() || el.scrollTop < 0 || el.scrollTop > el.scrollHeight - el.clientHeight))
      return undefined
    // Taken before the box changes height, which would clamp the position.
    const target = viewTop(el)
    const by = rowsShift.value
    settlingTop = target
    rowsShift.value = 0
    return nextTick(() => {
      settlingTop = null
      el.scrollTop = target
      scrolledTo = el.scrollTop
      // The rows stand where the reader saw them: no row moved.
      if (seen)
        seen = { ...seen, scrollTop: seen.scrollTop + by }
      logList('debug', 'list.anchor_corrected', { reason: 'rows_shift_settled', delta_px: Math.round(by) })
    })
  }

  /** The virtualizer exists: what reads it may. */
  const created = shallowRef(false)
  /** Where the view is and how tall, as the scroller says it now. */
  function view(instance: Virtualizer<HTMLElement, HTMLElement>): { offset: number, height: number } {
    const el = scrollContainer.value
    return {
      offset: el ? viewTop(el) : instance.scrollOffset ?? 0,
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
      // The virtualizer places the view in the list: on the scroller, below
      // the rows' shift. The position is read when it is reported: the
      // virtualizer also reports, once a scroll ends, the one it read last.
      observeElementOffset: (instance, report) =>
        observeElementOffset(instance, (offset, isScrolling) =>
          report(scrollContainer.value ? viewTop(scrollContainer.value) : offset + rowsShift.value, isScrolling)),
      scrollToFn: (offset, options, instance) => {
        // The list scrolls itself: what it held for the reader's scroll is
        // superseded, and the rows go back to their places first, as the
        // position it writes is theirs.
        pendingScrollBy = 0
        pendingFrom = null
        const settled = settleShift(true)
        if (settled) {
          void settled.then(() => {
            elementScroll(offset, options, instance)
            scrolledTo = scrollContainer.value?.scrollTop ?? Number.NaN
          })
          return
        }
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
        noteRowWidth(element, entry)
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

  const virtualItems = keptComputed(() => {
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
  // A click that opened or folded nothing leaves following as it was, and the
  // view catches up with what arrived while it held: a click on a copy button,
  // or on a reply's text while it streams, which grows the row by itself. The
  // reader changed a row when it changed height and a control or a <details>
  // it held then opened or folded (`aria-expanded`, `open`), or the control
  // they clicked gave way to what it opened, as Edit gives way to its editor;
  // never by the row's height alone, nor by controls that came since, as the
  // steps that join a run.
  let readerLayoutUntil = 0
  let readerLayoutTimer: ReturnType<typeof setTimeout> | undefined
  /**
   * The rows the reader acted on while the view holds, each with its height
   * and what it held open (`disclosures`) when first acted on.
   */
  const readerRows = new Map<HTMLElement, { height: number, disclosures: Map<Element, string | null>, controls: Set<Element> }>()
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
      readerRows.set(row, { height: row.getBoundingClientRect().height, disclosures: disclosures(row), controls: new Set() })
    const clicked = target?.closest('button, summary, [role="button"]')
    if (row && clicked)
      readerRows.get(row)!.controls.add(clicked)
    const control = target?.closest('button, summary, a, [role="button"]') ?? target
    readerControl = control ? { element: control, top: control.getBoundingClientRect().top } : null
    readerLayoutUntil = performance.now() + READER_LAYOUT_MS
    clearTimeout(readerLayoutTimer)
    readerLayoutTimer = setTimeout(() => {
      readerLayoutTimer = undefined
      const el = scrollContainer.value
      const resized = [...readerRows].some(([row, was]) =>
        Math.abs(row.getBoundingClientRect().height - was.height) > RESIZED_PX
        && ([...was.disclosures].some(([element, state]) => element.isConnected && openState(element) !== state)
          || [...was.controls].some((control) => !control.isConnected)))
      readerRows.clear()
      readerControl = null
      if (!el)
        return
      const dist = distanceFromBottom(el)
      endDistance.value = dist
      if (resized || !shouldAutoScroll.value)
        follow(dist < AT_END_PX, 'reader_changed_layout')
      else
        glideToEnd()
    }, READER_LAYOUT_MS)
  }
  /**
   * The view stops holding for what the reader opened, as when they then go
   * to the end: their later act decides where the view is.
   */
  function endHold(): void {
    clearTimeout(readerLayoutTimer)
    readerLayoutTimer = undefined
    readerLayoutUntil = 0
    readerRows.clear()
    readerControl = null
  }
  onScopeDispose(endHold)

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
    virtualizer.value.scrollOffset = viewTop(el) + moved
    el.scrollTop += moved
    scrolledTo = el.scrollTop
    scrollOffset.value = viewTop(el)
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

  /** The reader goes to the end, by the arrow or by sending: the list follows it from there, and glides to it. */
  function scrollToBottomExplicit() {
    endHold()
    follow(true, 'jump_to_end')
    glideToEnd()
  }

  /**
   * What covers the list's end grew, or the list got shorter, as the dock or
   * an on-screen keyboard does: the view stays at the end in the same frame,
   * so the latest line is never hidden for one.
   */
  function keepEnd() {
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
        readerTakes()
        if (e.deltaY < 0)
          pendingIntent = 'up'
        if (e.deltaY > 0)
          pendingIntent = 'down'
      }
      const onTouchStart = (e: TouchEvent) => {
        touchStartY = e.touches[0]?.clientY ?? 0
        touching = true
        readerTakes()
      }
      const onTouchEnd = (e: TouchEvent) => {
        if (!touching || e.touches.length > 0)
          return
        touching = false
        // A touch that scrolled nothing sends no scroll event to settle the shift.
        endMomentumLater()
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
        if (SCROLL_KEYS.has(e.key))
          readerTakes()
        if (e.key === 'Enter' || e.key === ' ')
          readerChangesLayout(e)
      }
      // A mouse or a pen held on the list or on its scrollbar, which stands
      // beside the scroller in its scroll area (`ScrollArea`), holds the
      // list as a finger does: the reader drags the bar or selects text.
      const area = el.parentElement ?? el
      const onPointerDown = (e: PointerEvent) => {
        if (e.pointerType === 'touch')
          return
        touching = true
        readerTakes()
        window.addEventListener('pointerup', onPointerUp, { capture: true })
        window.addEventListener('pointercancel', onPointerUp, { capture: true })
      }
      const onPointerUp = () => {
        window.removeEventListener('pointerup', onPointerUp, { capture: true })
        window.removeEventListener('pointercancel', onPointerUp, { capture: true })
        touching = false
        endMomentumLater()
      }
      area.addEventListener('pointerdown', onPointerDown, { capture: true })
      el.addEventListener('click', readerChangesLayout, { capture: true })
      el.addEventListener('keydown', onKeydown, { capture: true })
      el.addEventListener('wheel', onWheel, { passive: true })
      el.addEventListener('touchstart', onTouchStart, { passive: true })
      el.addEventListener('touchmove', onTouchMove, { passive: true })
      // The finger lifts where the window hears it: the row it touched can
      // leave the list as the list scrolls, and its touch's end with it.
      window.addEventListener('touchend', onTouchEnd, { passive: true, capture: true })
      window.addEventListener('touchcancel', onTouchEnd, { passive: true, capture: true })
      onCleanup(() => {
        area.removeEventListener('pointerdown', onPointerDown, { capture: true })
        window.removeEventListener('pointerup', onPointerUp, { capture: true })
        window.removeEventListener('pointercancel', onPointerUp, { capture: true })
        window.removeEventListener('touchend', onTouchEnd, { capture: true })
        window.removeEventListener('touchcancel', onTouchEnd, { capture: true })
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
   * The list grew or shrank at its end: a reader who follows it sees the view
   * glide to its end from the next frame (`glideToEnd`). Rows the virtualizer measures change
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
    glideToEnd()
  }

  /**
   * The list has been painted where it was placed: from then on, what it
   * grows by at its end is content that arrived, which the view glides to,
   * while before it the rows' first measurements only correct the place.
   */
  let shownPlaced = false
  watch(isRestored, (placed) => {
    shownPlaced = false
    if (placed)
      requestAnimationFrame(() => requestAnimationFrame(() => {
        shownPlaced = isRestored.value
      }))
  }, { flush: 'sync' })

  /**
   * The view following the end scrolls to it a little in each frame, the
   * scroll alone moving the rows, as use-stick-to-bottom does. Rows drawn
   * from where they stood while the view jumps under them would have to
   * land in the same frame as the jump, which iOS Safari paints a frame
   * apart: the rows drop by the growth for a frame, then come back.
   *
   * The glide is the list's own scroll: it ends no following, and moves no
   * row (`seen` goes with it). It stops when the reader touches the list,
   * scrolls it, or stops following, and resumes when their scroll ends.
   */
  let glideFrame = 0
  let glideAt = 0
  /**
   * The glide runs: every scroll meanwhile is the list's own, its glide or
   * its corrections, until the reader acts on the list (`readerTakes`).
   */
  let gliding = false
  function glideToEnd(): void {
    furthestDistanceSinceDisengage = 0
    if (glideFrame !== 0)
      return
    // An end further than a view away is jumped to, as nobody follows a
    // glide that long (`moveRows` moves no row that far either).
    const el = scrollContainer.value
    if (!el || !shownPlaced || prefersReducedMotion() || document.hidden || distanceFromBottom(el) > el.clientHeight) {
      scrollToBottom()
      return
    }
    glideAt = performance.now()
    gliding = true
    glideFrame = requestAnimationFrame(glide)
  }
  function glide(now: number): void {
    glideFrame = 0
    gliding = false
    const el = scrollContainer.value
    if (!el || !shouldAutoScroll.value || holdsView() || readerScrolls())
      return
    if (rowsShift.value !== 0) {
      void settleShift(true)
      glideAt = now
      gliding = true
      glideFrame = requestAnimationFrame(glide)
      return
    }
    const end = el.scrollHeight - el.clientHeight
    const left = end - el.scrollTop
    if (left < 1)
      return
    gliding = true
    const step = Math.min(left, Math.max(1, Math.round(left * (1 - Math.exp(-(now - glideAt) / GLIDE_MS)))))
    glideAt = now
    el.scrollTop += step
    scrolledTo = el.scrollTop
    if (seen)
      seen = scrolledRows(seen, el.scrollTop)
    glideFrame = requestAnimationFrame(glide)
  }
  /** The reader acts on the list: the glide stops, and what scrolls next is theirs. */
  function readerTakes(): void {
    cancelAnimationFrame(glideFrame)
    glideFrame = 0
    gliding = false
  }
  onScopeDispose(() => cancelAnimationFrame(glideFrame))
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
    if (reflowing) {
      seen = null
      placeAcrossReflow()
      return
    }
    if (!shouldAutoScroll.value)
      remeasureReadAt()
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
    if (reflowing && el) {
      scrollPending()
      stopRowMoves(el)
      if (keepsEnd)
        scrollToBottom()
      endDistance.value = distanceFromBottom(el)
    } else {
      scrollPending()
      if (el && seen)
        moveRows(el, seen, placedHeights(), shouldAutoScroll.value)
    }
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
        if (reflowing || holdsView())
          return false
        // The row whose part a reveal placed grows below that part, as its
        // fold opens: the reveal places the part again instead.
        if (placingPart() && item.key === revealing?.key)
          return false
        const offset = currentInstance.scrollOffset ?? 0
        // Following, a row that reaches into the view grows below what the
        // reader sees of it, as streamed text does: the view glides to the
        // end instead (followEnd). What it grew above the text it ends with,
        // as a formula drawn late does, would push what the reader reads
        // down, and is made up at once, as for a row above the view.
        if (shouldAutoScroll.value && shownPlaced && item.start + item.size > offset) {
          const row = item.start < offset ? currentInstance.elementsCache.get(item.key) : undefined
          const above = row ? tailDisplaced(row, settledTails.get(String(item.key))) : 0
          if (above > RESIZED_PX) {
            resizeCorrections += 1
            resizeCorrectedPx += Math.round(above)
            followRowsAbove(above)
          }
          return false
        }
        const isStreamingTail = item.index === blocks.value.length - 1
        const allowCorrection = shouldAutoScroll.value || !isStreamingTail
        const adjusts = allowCorrection && item.start < offset
        if (adjusts) {
          // Many a second as rows are measured: counted, and told with the scroll that follows.
          resizeCorrections += 1
          resizeCorrectedPx += Math.round(_delta)
          followRowsAbove(_delta)
        }
        return false
      }
    },
    { immediate: true },
  )

  // The list got wider or narrower, as when a panel beside it opens or the
  // window is resized: its text wraps anew and every row changes height at
  // once. That is no content coming or going, so no row moves, and the
  // reader's place holds in the same frame, as a browser keeps a page's
  // place through a resize (CSS scroll anchoring): the end for a reader who
  // follows it, otherwise the paragraph at the view's top, where it stood. A
  // paragraph the view's top cuts keeps the same share of it above the view,
  // so the words at the top stay about the same as it wraps into more lines
  // or fewer. A row's own growth would not do: a long reply that starts
  // above the view grows below it too, which would carry the view past what
  // the reader read.
  //
  // Every row spans the list's width, so a row measured at another width
  // than before says the list's width changed. The paragraph's place in its
  // row is read then, at the new width, before the update moves any row: the
  // update may draw it no more. Its row's new start then places the view
  // before the update draws the rows (`seeRows`), so the update draws the
  // rows the view will show. It holds for the frame: the rows that the new
  // places bring into view are measured in it too.
  /** The width each row was last measured at. */
  const rowWidths = new WeakMap<Element, number>()
  /** The list's width changed in this frame, and its rows are laid out anew. */
  let reflowing = false
  /** The reader follows the end through the reflow. */
  let keepsEnd = false
  /**
   * The paragraph, or other block of a row, at the view's top, as of the last
   * scroll or placing: its row's key and its place among the children at
   * each depth (a message drawn anew as it streams replaces its elements,
   * not their places), how far below the view's top it stood, and the share
   * of it above the view's top, when the view's top cut it.
   */
  let readAt: { key: string, path: number[], offset: number, above: number } | null = null
  /**
   * Where the reader's paragraph is to stand through the reflow: below its
   * row's start, with the rows' box's own place, and below the view's top.
   */
  let reflowAt: { key: string, inRow: number, offset: number } | null = null
  function noteRowWidth(row: Element, entry: ResizeObserverEntry | undefined): void {
    const width = entry?.borderBoxSize?.[0]?.inlineSize ?? row.getBoundingClientRect().width
    const was = rowWidths.get(row)
    if (width <= 0)
      return
    rowWidths.set(row, width)
    if (was === undefined || Math.abs(width - was) < 0.5 || !isRestored.value || reflowing)
      return
    reflowing = true
    keepsEnd = shouldAutoScroll.value && !holdsView()
    reflowAt = keepsEnd ? null : readingPlace()
    requestAnimationFrame(() => {
      reflowing = false
      reflowAt = null
    })
  }
  /** Where the reader's paragraph stands in its row now, at the new width, and where it is to stand in the view. */
  function readingPlace(): { key: string, inRow: number, offset: number } | null {
    const el = scrollContainer.value
    const at = readAt
    const row = el && at ? el.querySelector<HTMLElement>(`[data-row-key="${CSS.escape(at.key)}"]`) : null
    if (!row || !at)
      return null
    const { block, reached } = blockOf(row, at.path)
    const box = block.getBoundingClientRect()
    // The rows stand in a box that may start below the scrolled content's top.
    const rowsTop = (row.offsetParent ?? el!).getBoundingClientRect().top - el!.getBoundingClientRect().top + el!.scrollTop
    return {
      key: at.key,
      inRow: rowsTop + box.top - row.getBoundingClientRect().top,
      // A paragraph that is gone leaves its nearest block, by the place the
      // paragraph had in the view, which is close by.
      offset: reached && at.above > 0 ? -at.above * box.height : at.offset,
    }
  }
  /**
   * Places the view for the rows' new heights, before the update draws them:
   * at the end, or where the reader's paragraph stands where it stood.
   */
  function placeAcrossReflow(): void {
    const el = scrollContainer.value
    if (!el)
      return
    const instance = virtualizer.value
    const now = instance.scrollOffset ?? viewTop(el)
    let target: number | null = null
    if (keepsEnd) {
      target = Math.max(0, instance.getTotalSize() - el.clientHeight)
    } else if (reflowAt) {
      const index = blocks.value.findIndex((block) => keyOf(block) === reflowAt!.key)
      const start = instance.measurementsCache[index]?.start
      if (start !== undefined)
        target = start + reflowAt.inRow - reflowAt.offset
    }
    if (target !== null && Math.abs(target - now) >= 0.5)
      followRowsAbove(target - now)
  }
  /**
   * The deepest block of `row` that reaches below `y`, the view's top, by its
   * place among the children at each depth: a paragraph, an item or a code
   * block of a message, never a run of text inside one, which wraps
   * elsewhere at another width, nor what stands out of the flow, as a
   * message's actions over its foot. When the view's top leaves less than
   * half of that block in view and what follows it starts above `bottom`, the
   * view's bottom, the reader reads what follows: the next block at the
   * deepest depth that has one, or, with `last`, the next row. The children
   * of a block follow one another down the page, so each depth is searched
   * by halves.
   */
  function blockAt(row: HTMLElement, y: number, bottom: number): { path: number[], last: boolean } {
    const path: number[] = []
    const chain: Element[] = [row]
    let at: Element = row
    for (;;) {
      const children = at.children
      let low = 0
      let high = children.length
      while (low < high) {
        const middle = (low + high) >> 1
        if (children[middle]!.getBoundingClientRect().bottom > y)
          high = middle
        else
          low = middle + 1
      }
      const index = inFlow(children, low)
      const next = children[index]
      if (!next || getComputedStyle(next).display.startsWith('inline'))
        break
      path.push(index)
      chain.push(next)
      at = next
    }
    const box = at.getBoundingClientRect()
    if (at === row || box.top >= y || box.bottom - y >= box.height / 2 || box.bottom >= bottom)
      return { path, last: false }
    for (let depth = path.length - 1; depth >= 0; depth -= 1) {
      const siblings = chain[depth]!.children
      const after = inFlow(siblings, path[depth]! + 1)
      if (after < siblings.length)
        return { path: [...path.slice(0, depth), after], last: false }
    }
    return { path, last: true }
  }
  /** The block of `row` at `path`, or how far the path still leads. */
  function blockOf(row: Element, path: number[]): { block: Element, reached: boolean } {
    let block = row
    for (const index of path) {
      const child = block.children[index]
      if (!child)
        return { block, reached: false }
      block = child
    }
    return { block, reached: true }
  }
  /**
   * Reads again where the reader's block stands, as content arrives while
   * the reader does not scroll: a paragraph that streams grows below the
   * view's top, and its share above it shrinks.
   */
  function remeasureReadAt(): void {
    const el = scrollContainer.value
    const at = readAt
    const row = el && at ? el.querySelector<HTMLElement>(`[data-row-key="${CSS.escape(at.key)}"]`) : null
    if (!el || !at || !row)
      return
    const { block, reached } = blockOf(row, at.path)
    if (!reached)
      return
    const box = block.getBoundingClientRect()
    const offset = box.top - el.getBoundingClientRect().top
    readAt = { ...at, offset, above: offset < 0 && box.height > 0 ? -offset / box.height : 0 }
  }
  /** The first of `children` from `from` on that stands in the flow and has a height. */
  function inFlow(children: HTMLCollection, from: number): number {
    let index = from
    while (index < children.length) {
      const style = getComputedStyle(children[index]!)
      if (style.position !== 'absolute' && style.position !== 'fixed' && children[index]!.getBoundingClientRect().height > 0)
        break
      index += 1
    }
    return index
  }

  let lastAnchor: ViewAnchor | null = null

  function updateAnchor() {
    const el = scrollContainer.value
    if (!el || blocks.value.length === 0)
      return
    const st = viewTop(el)
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
        const bottom = containerTop + el.clientHeight
        let read = anchorEl
        let found = blockAt(read, containerTop, bottom)
        const next = found.last ? el.querySelector<HTMLElement>(`[data-index="${item.index + 1}"]`) : null
        if (next) {
          read = next
          found = blockAt(read, containerTop, bottom)
        }
        const path = found.path
        const box = blockOf(read, path).block.getBoundingClientRect()
        const offset = box.top - containerTop
        readAt = { key: read.dataset['rowKey']!, path, offset, above: offset < 0 && box.height > 0 ? -offset / box.height : 0 }
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
    const top = viewTop(el)
    const bottom = top + el.clientHeight
    const first = rows.findIndex((row) => row.end > top - reach)
    const last = rows.findLastIndex((row) => row.start < bottom + reach)
    return first < 0 || last < 0 ? null : { first, last }
  }

  function onScroll() {
    const el = scrollContainer.value
    if (el) {
      scrollOffset.value = viewTop(el)
      // The list's own scroll while it glides: the reader neither scrolls nor
      // leaves the end.
      const glided = gliding && pendingIntent === null
      if (shiftsRows && !glided) {
        endMomentumLater()
        // The view reached the scroller's top while rows are shifted: above
        // it are rows it cannot scroll to, or a gap where they are drawn
        // below their places. The reader's scroll stops there in any case.
        const top = el.scrollTop
        if (top >= 0 && (rowsShift.value > 0 ? top < 1 : top + rowsShift.value < 0))
          void settleShift(true)
      }
      // What the reader scrolled is no move of any row: a place kept from before
      // it stands further along.
      if (seen && Math.abs(el.scrollTop - scrolledTo) >= 1)
        seen = scrolledRows(seen, el.scrollTop)
      const dist = distanceFromBottom(el)
      endDistance.value = dist
      if (!glided || pendingIntent !== null)
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

  /** Ends the reader's scroll once no scroll event came for `MOMENTUM_END_MS`, and settles the rows' shift then. */
  function endMomentumLater(): void {
    clearTimeout(momentumEnd)
    momentumEnd = setTimeout(() => {
      momentumEnd = undefined
      settleShift()
      if (shouldAutoScroll.value && shownPlaced)
        glideToEnd()
    }, MOMENTUM_END_MS)
  }
  onScopeDispose(() => clearTimeout(momentumEnd))

  /** Where the current scroll began, and the timer that ends it once the view stays still. */
  let scrolledFrom = 0
  let scrollSettle: ReturnType<typeof setTimeout> | undefined
  function scrollOffsetAtStart(el: HTMLElement): number {
    return lastScrollTop ?? viewTop(el)
  }
  /** The scroll position the list last wrote a line at, or knew at the last settle. */
  let lastScrollTop: number | undefined
  /** Writes where a scroll stopped: from and to, the rows in view, and what the list corrected by itself meanwhile. */
  function writeScrolled(): void {
    const el = scrollContainer.value
    if (!el)
      return
    const to = Math.round(viewTop(el))
    const from = Math.round(scrolledFrom)
    lastScrollTop = viewTop(el)
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

    const anchor = persistedState?.anchor
    if (!anchor) {
      scrollToBottom()
      scrollOffset.value = el.scrollTop
      lastScrollTop = el.scrollTop
      logList('info', 'list.placed', { how: 'end', saved: persistedState !== undefined, scroll_top_px: Math.round(el.scrollTop) })
      nextTick(() => updateAnchor())
      return
    }

    const anchorIndex = blocks.value.findIndex((b) => b.id === anchor.blockId)
    if (anchorIndex < 0) {
      scrollToBottom()
      scrollOffset.value = el.scrollTop
      lastScrollTop = el.scrollTop
      logList('info', 'list.placed', { how: 'end', saved: true, saved_row_held: false, scroll_top_px: Math.round(el.scrollTop) })
      nextTick(() => updateAnchor())
      return
    }

    el.scrollTop = anchor.scrollTop
    lastScrollTop = el.scrollTop
    // The reader left the end: the list does not follow it, whether or not
    // the restored position moves the scroller and says so by a scroll event.
    endDistance.value = distanceFromBottom(el)
    follow(false, 'saved_place')
    logList('info', 'list.placed', { how: 'saved_place', saved: true, saved_row_held: true, row: anchorIndex, scroll_top_px: Math.round(el.scrollTop) })
    correctUntilConverged(el, anchorIndex, anchor.offsetPx, 3)
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
      const target = (virtualizer.value.scrollOffset ?? viewTop(el)) + moved - anchor.start
      virtualizer.value.scrollOffset = target
      el.scrollTop = target - rowsShift.value
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
    const following = isRestored.value && shouldAutoScroll.value
    if (!lastAnchor && !following)
      return undefined
    return {
      anchor: following || !lastAnchor ? null : {
        blockId: lastAnchor.blockId,
        anchorIndex: lastAnchor.anchorIndex,
        offsetPx: lastAnchor.offsetPx,
        scrollTop: scrollContainer.value ? viewTop(scrollContainer.value) : lastAnchor.scrollTop
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
    keepEnd,
    reveal,
    rowsNear,
    onScroll,
    /** Where the view starts in the list, as `scrollOffset` says after a scroll. */
    viewTop: (): number => (scrollContainer.value ? viewTop(scrollContainer.value) : 0),
    /** How far above their places the rows and their box are drawn (iOS, while the reader scrolls). */
    rowsShift: readonly(rowsShift),
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
