<script setup lang="ts" generic="R extends Row">
import { computed, nextTick, onMounted, ref, useId, watch } from 'vue'
import { useResizeObserver } from '@vueuse/core'
import { defaultRangeExtractor, useVirtualizer, type Range } from '@tanstack/vue-virtual'
import { useTypeSelect } from '../composables/useTypeSelect'
import { activeRow } from '../ui/active-row'
import DropOutline from '../ui/DropOutline.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import Tooltip from '../ui/Tooltip.vue'
import TypeSelectHint from '../ui/TypeSelectHint.vue'
import TreeRow from './TreeRow.vue'
import { usePhone } from '../ui/phone'
import {
  TREE_ROW_PX,
  TREE_ROW_PHONE_PX,
  indexTree,
  revealTreeRow,
  stickyTreeRows,
  treeKeyEffect,
  type TreeDropTarget,
  type TreeRow as Row
} from './tree'
import type { SentenceText } from '../ui/ui-text'
import { vContextMenu, type ContextMenuValue } from '../ui/context-menu'

/**
 * A tree of rows laid out by its host: a caption, then the rows, on the editor's
 * surface. A click on a row asks the host to activate it (fold a directory, open
 * a file).
 *
 * Only the rows in view are drawn, with a few around them, so a directory of
 * any size unfolds and scrolls at once. Every row is one pitch tall, so where a
 * row sits is its index times the pitch: the pinned stack, a drop's block and
 * bringing a row into view are reckoned from the rows, never measured from the
 * page. The row the keyboard is on and the row whose menu is open stay drawn,
 * as the tree's active descendant and the menu's anchor. Each row tells its
 * level and its place among its siblings, since the page holds only some.
 *
 * Pinning:
 *
 * - The caption stays pinned.
 * - Under it pin the directories the rows at the top sit in, each while its own
 *   row has scrolled out above and its contents have not, so the way to what is
 *   in view stays in sight and goes once the tree is past it.
 * - A pinned directory acts as its row, and once it folds its row rests where
 *   the pinned copy was.
 *
 * Slots and menus:
 *
 * - The rows are dressed through the slots `mark`, `name` and `trailing`, given
 *   the row.
 * - `tooltip` covers a row with a hint, and `empty` fills the tree while it has
 *   no rows.
 * - Each row declares the context menu the host gives it (`rowMenu`), and the
 *   tree's empty space the one the host gives no row.
 *
 * Where a drag would drop (`dropTarget`) lights under a dashed line: a directory
 * row with the rows it holds, its pinned copies too, or the whole tree. The host
 * follows the drag itself and asks which row an event happened in (`rowAt`).
 *
 * Keyboard: the tree takes it, focused by Tab or by a click on a row. A cursor
 * marks the row the keys act on, starting at the selected row, else the first.
 *
 * - Up, Down, Home and End move the cursor.
 * - Right unfolds a folded directory. Left folds an unfolded one or goes to the
 *   directory a row is in.
 * - Enter activates its row as a click does.
 * - Typing a name moves the cursor to the first row shown that starts with it
 *   (`useTypeSelect`), opening nothing.
 */
const props = defineProps<{
  rows: readonly R[]
  /** The tree's name, heading it as a plain caption. */
  caption: string
  /** What the caption stands for, on hover. */
  captionTitle?: SentenceText
  /** The selected row, by path. */
  selected: string | null
  /** The row whose menu is open, by path; it keeps its hover look until the menu closes. */
  menuRow?: string | null
  /** What a drag over the tree would drop into, lit while it is there. */
  dropTarget?: TreeDropTarget | null
  tooltip?: (row: R) => string
  /** A row's context menu, and given no row the menu of the tree's empty space. */
  rowMenu?: (row: R | null) => ContextMenuValue
}>()

const emit = defineEmits<{
  activate: [row: R]
}>()

defineSlots<{
  /** In place of the caption's plain text; it takes the caption's room itself. */
  caption?(): unknown
  /** After the caption, at the row's end. */
  captionTrailing?(): unknown
  mark?(props: { row: R }): unknown
  /** `highlight`: the letters of the name to mark, as a type-select query's prefix. */
  name?(props: { row: R; highlight: readonly number[] | undefined }): unknown
  trailing?(props: { row: R }): unknown
  empty?(): unknown
  /** Under the last row, when there is something to say about the list itself. */
  after?(): unknown
}>()

// The pinned stack: measured from the rows' positions on every scroll and layout.
// Its first slot starts under the viewport padding, the caption and one gap,
// exactly where the first row starts, so a row and its pinned copy coincide.
// On a phone every row, the caption's included, is a finger's height.
const phone = usePhone()
const rowPx = computed(() => (phone.value ? TREE_ROW_PHONE_PX : TREE_ROW_PX))
const stackTopPx = computed(() => 4 + rowPx.value + 1)
const pitchPx = computed(() => rowPx.value + 1)
const scrollArea = ref<InstanceType<typeof ScrollArea> | null>(null)
// The row each element stands for, listed or pinned, to tell where an event happened.
const pathOfEl = new WeakMap<Node, string>()
const treeIndex = computed(() => indexTree(props.rows))
/** The index of the row at `path`, or -1 when no row is. */
function indexOf(path: string | null | undefined): number {
  return path == null ? -1 : (treeIndex.value.indexOf.get(path) ?? -1)
}
/** Where row `at` starts in the scrolled content. */
function rowTop(at: number): number {
  return stackTopPx.value + at * pitchPx.value
}
const stickyPaths = ref<string[]>([])
const stickyOffset = ref(0)
const scrolled = ref(false)
// The selected row loses its fill once any of it is under the stack: a sliver
// of highlight at the stack's edge would read as a line.
const selectedUnderStack = ref(false)
const stickyRows = computed(() => stickyPaths.value.flatMap((path) => {
  const row = props.rows[indexOf(path)]
  return row ? [row] : []
}))

/** A template ref's element: the element itself, or a component's root. */
function elementOf(el: unknown): HTMLElement | null {
  if (el instanceof HTMLElement)
    return el
  if (el && typeof el === 'object' && '$el' in el && el.$el instanceof HTMLElement)
    return el.$el
  return null
}

function bindRow(path: string, el: unknown): void {
  const element = elementOf(el)
  if (element)
    pathOfEl.set(element, path)
}

/** The row an event happened in, listed or pinned; null for the tree's empty space. */
function rowAt(target: EventTarget | null): R | null {
  for (let node = target instanceof Node ? target : null; node; node = node.parentNode) {
    const path = pathOfEl.get(node)
    if (path !== undefined)
      return props.rows[indexOf(path)] ?? null
  }
  return null
}

function updateSticky(): void {
  const viewport = scrollArea.value?.el
  if (!viewport) {
    return
  }
  scrolled.value = viewport.scrollTop > 0
  const stack = stickyTreeRows(props.rows, treeIndex.value, viewport.scrollTop, stackTopPx.value, rowPx.value)
  stickyPaths.value = stack.paths
  stickyOffset.value = stack.offset
  const stackBottom = viewport.scrollTop + stackTopPx.value + stack.paths.length * pitchPx.value + stack.offset
  const selected = indexOf(props.selected)
  selectedUnderStack.value = selected >= 0 && rowTop(selected) < stackBottom
}

// A drop into a row lights the row and the rows it holds: one box over the
// listed ones, placed from their positions like the stack, and each pinned copy.
const dropBlock = computed(() => {
  const first = props.dropTarget?.kind === 'row' ? indexOf(props.dropTarget.path) : -1
  return first < 0 ? null : { first, last: treeIndex.value.end[first]! }
})
/** Whether row `at` is in the block a drop would go into. */
function inDrop(at: number): boolean {
  const block = dropBlock.value
  return block !== null && at >= block.first && at <= block.last
}
const dropBox = ref<{ top: number; height: number } | null>(null)

function measureDrop(): void {
  const viewport = scrollArea.value?.el
  const block = dropBlock.value
  dropBox.value = viewport && block
    ? { top: rowTop(block.first) - viewport.scrollTop, height: (block.last - block.first) * pitchPx.value + rowPx.value }
    : null
}

/** Everything placed from the rows' positions: the pinned stack, and where a drop would go. */
function layout(): void {
  updateSticky()
  measureDrop()
}

/** Scrolls a row to the top of the view, under the caption, for a host that sets up a scrolled state. */
function scrollToRow(path: string): void {
  const viewport = scrollArea.value?.el
  const at = indexOf(path)
  if (viewport && at >= 0) {
    viewport.scrollTop = rowTop(at) - stackTopPx.value
  }
}

// The row to bring into view once the hidden tree it was asked of is shown.
let revealWhenShown: string | null = null

/**
 * Brings a row into view, leaving the tree still if it already is: a row
 * above the view, or under its pinned ancestors, comes to rest just below
 * them; one below the view comes up to its bottom edge.
 */
function revealRow(path: string): void {
  const viewport = scrollArea.value?.el
  const at = indexOf(path)
  const row = props.rows[at]
  revealWhenShown = null
  if (!viewport || !row) {
    return
  }
  // A hidden tree has no height to bring the row into: it does once shown.
  if (viewport.clientHeight === 0) {
    revealWhenShown = path
    return
  }
  const top = revealTreeRow(viewport, { depth: row.depth, top: rowTop(at) }, stackTopPx.value, rowPx.value)
  if (top !== null) {
    viewport.scrollTop = top
  }
}

/**
 * A click on a pinned row acts as the row would. Its directory folds, so the
 * copy's slot would fill with what lies under it; the row itself comes to
 * rest there instead, once the rows have changed.
 */
function activatePinned(row: R): void {
  activateRow(row)
  void nextTick(() => revealRow(row.path))
}

// The keyboard: the row the keys act on, by path; until a key or a click
// moves it, or once its row has gone, the selected row stands in.
const treeEl = ref<HTMLElement | null>(null)
const treeId = useId()
const focused = ref(false)
const cursor = ref<string | null>(null)
const cursorPath = computed(() => activeRow(props.rows.map((row) => row.path), cursor.value, props.selected))
const cursorIndex = computed(() => indexOf(cursorPath.value))

// The rows drawn: those in view and a few around them, and the two that must stay
// drawn. The kept rows are read here, not in the extractor, so a move of either
// gives the virtualizer a new extractor and it draws again.
const virtualizer = useVirtualizer(computed(() => {
  const kept = [cursorIndex.value, indexOf(props.menuRow)].filter((at) => at >= 0)
  return {
    count: props.rows.length,
    getScrollElement: () => scrollArea.value?.el ?? null,
    estimateSize: () => pitchPx.value,
    scrollMargin: stackTopPx.value,
    getItemKey: (at: number) => props.rows[at]?.path ?? at,
    overscan: 10,
    rangeExtractor: (range: Range) => [...new Set([...defaultRangeExtractor(range), ...kept])].sort((a, b) => a - b),
  }
}))
// A phone's rows are taller: every row moves.
watch(rowPx, () => virtualizer.value.measure())
/**
 * The drawn rows in the page's flow, each after the room of the rows skipped
 * before it, and the room of those after the last. In the flow, not placed one
 * by one, the browser keeps the rows in view where they are when rows come or go
 * above them, as it does for any page (scroll anchoring): the room is held by
 * siblings of the rows, whose changes it follows, never by a height of the list.
 */
const drawnRows = computed(() => {
  const pitch = pitchPx.value
  let next = 0
  const drawn = virtualizer.value.getVirtualItems().flatMap((item) => {
    const row = props.rows[item.index]
    if (!row)
      return []
    const before = (item.index - next) * pitch
    next = item.index + 1
    return [{ row, at: item.index, before }]
  })
  const after = props.rows.length - next
  return { drawn, after: after > 0 ? after * pitch - 1 : 0 }
})

function moveCursor(path: string): void {
  cursor.value = path
  void nextTick(() => revealRow(path))
}

const typeSelect = useTypeSelect({
  names: () => props.rows.map((row) => row.name),
  select: (index) => moveCursor(props.rows[index]!.path),
})

/** The letters to mark on a row's name: the typed prefix, on the cursor's row only. */
function highlightOf(row: R): readonly number[] | undefined {
  return row.path === cursorPath.value ? (typeSelect.prefix(row.name) ?? undefined) : undefined
}

/** A click on a row, listed or pinned, or Enter on it: the tree takes the keyboard there, and the row activates. */
function activateRow(row: R): void {
  cursor.value = row.path
  treeEl.value?.focus({ preventScroll: true })
  emit('activate', row)
}

function onKeydown(event: KeyboardEvent): void {
  // The tree's own keys; a control a slot put in a row keeps its own.
  if (event.target !== event.currentTarget)
    return
  if (typeSelect.keydown(event)) {
    event.preventDefault()
    return
  }
  if (event.altKey || event.metaKey || event.ctrlKey)
    return
  const effect = treeKeyEffect(props.rows, cursorPath.value, event.key)
  if (!effect)
    return
  event.preventDefault()
  if (effect.kind === 'cursor') {
    moveCursor(effect.path)
    return
  }
  const row = props.rows[indexOf(effect.path)]
  if (row)
    activateRow(row)
}

/** Moves the tree by `px`, for a host that sets up a scrolled state. */
function scrollBy(px: number): void {
  const viewport = scrollArea.value?.el
  if (viewport) {
    viewport.scrollTop += px
    layout()
  }
}

onMounted(layout)
// A tree laid out while hidden, as a frame's sidebar that slides in is, measured
// every row at the top: it measures them again once it is shown or resized.
useResizeObserver(() => scrollArea.value?.el, () => {
  layout()
  if (revealWhenShown !== null && (scrollArea.value?.el?.clientHeight ?? 0) > 0)
    revealRow(revealWhenShown)
})
watch([() => props.rows, () => props.selected, () => props.dropTarget], () => {
  void nextTick(layout)
})

defineExpose({ scrollToRow, revealRow, scrollBy, rowAt })
</script>

<template>
  <!-- The tree paints its own surface, the editor's, so the pinned stack matches it wherever it sits. -->
  <ScrollArea
    ref="scrollArea"
    class="h-full min-h-0 bg-surface-editor"
    viewport-class="p-1"
    @scroll="layout"
    v-context-menu="rowMenu?.(null)"
  >
    <!-- The pinned stack: the caption stays put; the directories under it
         slide up beneath the caption as the tree scrolls past them. -->
    <!-- Above the rows (whose transformed chevrons would otherwise paint through), below the
         scroll area's thumb; `isolate` keeps the caption's layering inside. The surface covers
         the padding too, so nothing shows through the gaps, and the stack takes the pointer, so
         the rows it covers get no hover or click through it. -->
    <div class="absolute inset-x-0 top-0 isolate z-[1] flex flex-col bg-surface-editor p-1 pb-0">
      <Tooltip
        tag="div"
        class="relative z-10 flex h-7 shrink-0 select-none items-center bg-surface-editor px-2 text-chrome font-medium text-fg-muted phone:h-11 phone:text-[15px]"
        :content="captionTitle"
      >
        <slot name="caption"><span class="min-w-0 flex-1 truncate">{{ caption }}</span></slot>
        <slot name="captionTrailing" />
      </Tooltip>
      <!-- Each pinned row in its own clip; only the deepest slides up as its
           directory leaves, its clip shrinking with it, while the rows above stay. -->
      <div class="flex flex-col gap-px bg-surface-editor pt-px">
        <div
          v-for="(row, index) in stickyRows"
          :key="row.path"
          class="relative overflow-hidden"
          :style="{ height: `${index === stickyRows.length - 1 ? Math.max(0, rowPx + stickyOffset) : rowPx}px` }"
        >
          <TreeRow
            :ref="(el) => bindRow(row.path, el)"
            :style="index === stickyRows.length - 1 ? { transform: `translateY(${stickyOffset}px)` } : undefined"
            :row="row"
            :selected="row.path === selected"
            :menu-open="row.path === menuRow"
            :tooltip="tooltip?.(row)"
            @activate="activatePinned(row)"
            v-context-menu="rowMenu?.(row)"
          >
            <template #mark><slot name="mark" :row="row" /></template>
            <template #name><slot name="name" :row="row" :highlight="undefined" /></template>
            <template #trailing><slot name="trailing" :row="row" /></template>
          </TreeRow>
          <div v-if="inDrop(indexOf(row.path))" class="pointer-events-none absolute inset-0 bg-drop-target" />
        </div>
      </div>
      <!-- Only scrolled rows need a fall-off below the stack that covers them. -->
      <div v-if="scrolled && rows.length > 0" class="pointer-events-none absolute inset-x-0 top-full h-2 bg-linear-to-b from-surface-editor to-transparent" />
    </div>
    <!-- The caption's room plus one gap; the pinned copy above covers it. -->
    <div class="shrink-0" :style="{ height: `${rowPx + 1}px` }" aria-hidden="true" />
    <div
      ref="treeEl"
      role="tree"
      :aria-label="caption"
      :aria-activedescendant="cursorIndex >= 0 ? `${treeId}-${cursorIndex}` : undefined"
      tabindex="0"
      class="flex min-h-full flex-col gap-px outline-none"
      @keydown="onKeydown"
      @focus="focused = true"
      @blur="focused = false"
    >
      <!-- The rows, one pitch each: the drawn ones, and the room of the others. -->
      <div v-if="rows.length > 0" class="flex shrink-0 flex-col">
        <template v-for="{ row, at, before } in drawnRows.drawn" :key="row.path">
          <div v-if="before > 0" class="shrink-0" :style="{ height: `${before}px` }" aria-hidden="true" />
          <TreeRow
            :id="`${treeId}-${at}`"
            :ref="(el) => bindRow(row.path, el)"
            :class="at < rows.length - 1 ? 'mb-px' : ''"
            :aria-level="row.depth + 1"
            :aria-posinset="treeIndex.position[at]"
            :aria-setsize="treeIndex.siblings[at]"
            :row="row"
            :selected="row.path === selected && !selectedUnderStack"
            :menu-open="row.path === menuRow"
            :cursor="focused && at === cursorIndex"
            :highlight="highlightOf(row)"
            :tooltip="tooltip?.(row)"
            @activate="activateRow(row)"
            v-context-menu="rowMenu?.(row)"
          >
            <template #mark><slot name="mark" :row="row" /></template>
            <template #name><slot name="name" :row="row" :highlight="highlightOf(row)" /></template>
            <template #trailing><slot name="trailing" :row="row" /></template>
          </TreeRow>
        </template>
        <div v-if="drawnRows.after > 0" class="shrink-0" :style="{ height: `${drawnRows.after}px` }" aria-hidden="true" />
      </div>
      <slot v-if="rows.length === 0" name="empty" />
      <slot v-else name="after" />
    </div>
    <!-- Over the listed rows and under the stack, so a block whose row has scrolled
         under it runs on beneath the pinned copies. -->
    <div
      v-if="dropBox"
      class="pointer-events-none absolute inset-x-1 rounded-md bg-drop-target"
      :style="{ top: `${dropBox.top}px`, height: `${dropBox.height}px` }"
    >
      <DropOutline radius="6px" />
    </div>
    <div v-if="dropTarget?.kind === 'tree'" class="pointer-events-none absolute inset-1 z-[1] rounded-md bg-drop-target">
      <DropOutline radius="6px" />
    </div>
    <TypeSelectHint :query="typeSelect.query.value" :matched="typeSelect.matched.value" />
  </ScrollArea>
</template>
