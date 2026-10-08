<script setup lang="ts" generic="R extends Row">
import { computed, nextTick, onMounted, ref, useId, watch } from 'vue'
import { useResizeObserver } from '@vueuse/core'
import { useTypeSelect } from '../composables/useTypeSelect'
import DropOutline from '../ui/DropOutline.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import TypeSelectHint from '../ui/TypeSelectHint.vue'
import TreeRow from './TreeRow.vue'
import {
  TREE_ROW_PITCH_PX,
  TREE_ROW_PX,
  revealTreeRow,
  stickyTreeRows,
  treeBlock,
  treeKeyEffect,
  type TreeDropTarget,
  type TreeRow as Row
} from './tree'
import type { SentenceText } from '../ui/ui-text'

/**
 * A tree of rows laid out by its host: a caption, then the rows, on the
 * editor's surface. A click on a row asks the host to activate it (fold a
 * directory, open a file). The caption stays pinned; under it pin the
 * directories the rows at the top sit in, each while its own row has
 * scrolled out above and its contents have not, so the way to what is in
 * view stays in sight and goes once the tree is past it. A pinned
 * directory acts as its row, and once it folds its row rests where the
 * pinned copy was. The rows are dressed through the slots `mark`, `name`
 * and `trailing`, given the row; `tooltip` covers a row with a hint;
 * `empty` fills the tree while it has no rows. A right-click asks the host
 * for a menu (`menu`), on a row or, given no row, on the tree's empty space;
 * a host with nothing to offer leaves the web browser's own.
 *
 * Where a drag would drop (`dropTarget`) lights under a dashed line: a
 * directory row with the rows it holds, its pinned copies too, or the whole
 * tree. The host follows the drag itself and asks which row an event
 * happened in (`rowAt`).
 *
 * The tree takes the keyboard, focused by Tab or by a click on a row. A
 * cursor marks the row the keys act on, starting at the selected row: Up,
 * Down, Home and End move it, Right unfolds a folded directory, Left folds
 * an unfolded one or goes to the directory a row is in, and Enter activates
 * its row as a click does. Typing a name moves the cursor to the first row
 * shown that starts with it (`useTypeSelect`), opening nothing.
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
}>()

const emit = defineEmits<{
  activate: [row: R]
  menu: [row: R | null, event: MouseEvent]
}>()

defineSlots<{
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
const STACK_TOP_PX = 4 + TREE_ROW_PX + 1
const scrollArea = ref<InstanceType<typeof ScrollArea> | null>(null)
const rowEls = new Map<string, HTMLElement>()
// The row each element stands for, pinned copies too, to tell where an event happened.
const pathOfEl = new WeakMap<Node, string>()
const stickyPaths = ref<string[]>([])
const stickyOffset = ref(0)
const scrolled = ref(false)
// The selected row loses its fill once any of it is under the stack: a sliver
// of highlight at the stack's edge would read as a line.
const selectedUnderStack = ref(false)
const stickyRows = computed(() => {
  const byPath = new Map(props.rows.map((row) => [row.path, row]))
  return stickyPaths.value.flatMap((path) => {
    const row = byPath.get(path)
    return row ? [row] : []
  })
})

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
  if (element) {
    rowEls.set(path, element)
    pathOfEl.set(element, path)
  } else {
    rowEls.delete(path)
  }
}

function bindPinned(path: string, el: unknown): void {
  const element = elementOf(el)
  if (element)
    pathOfEl.set(element, path)
}

/** The row an event happened in, listed or pinned; null for the tree's empty space. */
function rowAt(target: EventTarget | null): R | null {
  for (let node = target instanceof Node ? target : null; node; node = node.parentNode) {
    const path = pathOfEl.get(node)
    if (path !== undefined)
      return props.rows.find((row) => row.path === path) ?? null
  }
  return null
}

function updateSticky(): void {
  const viewport = scrollArea.value?.el
  if (!viewport) {
    return
  }
  scrolled.value = viewport.scrollTop > 0
  const stack = stickyTreeRows(
    props.rows,
    (path) => rowEls.get(path)?.offsetTop,
    viewport.scrollTop,
    STACK_TOP_PX,
  )
  stickyPaths.value = stack.paths
  stickyOffset.value = stack.offset
  const stackBottom = viewport.scrollTop + STACK_TOP_PX + stack.paths.length * TREE_ROW_PITCH_PX + stack.offset
  const selectedTop = props.selected === null ? undefined : rowEls.get(props.selected)?.offsetTop
  selectedUnderStack.value = selectedTop !== undefined && selectedTop < stackBottom
}

// A drop into a row lights the row and the rows it holds: one box over the
// listed ones, placed from their positions like the stack, and each pinned copy.
const dropBlock = computed(() =>
  props.dropTarget?.kind === 'row' ? treeBlock(props.rows, props.dropTarget.path) : null)
const dropPaths = computed(() => {
  const block = dropBlock.value
  return new Set(block ? props.rows.slice(block.first, block.last + 1).map((row) => row.path) : [])
})
const dropBox = ref<{ top: number; height: number } | null>(null)

function measureDrop(): void {
  const viewport = scrollArea.value?.el
  const block = dropBlock.value
  const first = block ? rowEls.get(props.rows[block.first]!.path) : undefined
  const last = block ? rowEls.get(props.rows[block.last]!.path) : undefined
  dropBox.value = viewport && first && last
    ? { top: first.offsetTop - viewport.scrollTop, height: last.offsetTop + last.offsetHeight - first.offsetTop }
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
  const el = rowEls.get(path)
  if (viewport && el) {
    viewport.scrollTop = el.offsetTop - STACK_TOP_PX
  }
}

/**
 * Brings a row into view, leaving the tree still if it already is: a row
 * above the view, or under its pinned ancestors, comes to rest just below
 * them; one below the view comes up to its bottom edge.
 */
function revealRow(path: string): void {
  const viewport = scrollArea.value?.el
  const el = rowEls.get(path)
  const row = props.rows.find((entry) => entry.path === path)
  if (!viewport || !el || !row) {
    return
  }
  const top = revealTreeRow(viewport, { depth: row.depth, top: el.offsetTop }, STACK_TOP_PX)
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
const cursorPath = computed(() => {
  const has = (path: string | null) => path !== null && props.rows.some((row) => row.path === path)
  if (has(cursor.value))
    return cursor.value
  return has(props.selected) ? props.selected : null
})
const cursorIndex = computed(() => props.rows.findIndex((row) => row.path === cursorPath.value))

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
  const row = props.rows.find((entry) => entry.path === effect.path)
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
useResizeObserver(() => scrollArea.value?.el, layout)
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
    @contextmenu="emit('menu', null, $event)"
  >
    <!-- The pinned stack: the caption stays put; the directories under it
         slide up beneath the caption as the tree scrolls past them. -->
    <!-- Above the rows (whose transformed chevrons would otherwise paint through), below the
         scroll area's thumb; `isolate` keeps the caption's layering inside. The surface covers
         the padding too, so nothing shows through the gaps, and the stack takes the pointer, so
         the rows it covers get no hover or click through it. -->
    <div class="absolute inset-x-0 top-0 isolate z-[1] flex flex-col bg-surface-editor p-1 pb-0">
      <div
        class="relative z-10 flex h-7 shrink-0 select-none items-center bg-surface-editor px-2 text-chrome font-medium text-fg-muted"
        :title="captionTitle"
      >
        <span class="min-w-0 flex-1 truncate">{{ caption }}</span>
        <slot name="captionTrailing" />
      </div>
      <!-- Each pinned row in its own clip; only the deepest slides up as its
           directory leaves, its clip shrinking with it, while the rows above stay. -->
      <div class="flex flex-col gap-px bg-surface-editor pt-px">
        <div
          v-for="(row, index) in stickyRows"
          :key="row.path"
          class="relative overflow-hidden"
          :style="{ height: `${index === stickyRows.length - 1 ? Math.max(0, TREE_ROW_PX + stickyOffset) : TREE_ROW_PX}px` }"
        >
          <TreeRow
            :ref="(el) => bindPinned(row.path, el)"
            :style="index === stickyRows.length - 1 ? { transform: `translateY(${stickyOffset}px)` } : undefined"
            :row="row"
            :selected="row.path === selected"
            :menu-open="row.path === menuRow"
            :tooltip="tooltip?.(row)"
            @activate="activatePinned(row)"
            @contextmenu.stop="emit('menu', row, $event)"
          >
            <template #mark><slot name="mark" :row="row" /></template>
            <template #name><slot name="name" :row="row" :highlight="undefined" /></template>
            <template #trailing><slot name="trailing" :row="row" /></template>
          </TreeRow>
          <div v-if="dropPaths.has(row.path)" class="pointer-events-none absolute inset-0 bg-drop-target" />
        </div>
      </div>
      <!-- Only scrolled rows need a fall-off below the stack that covers them. -->
      <div v-if="scrolled && rows.length > 0" class="pointer-events-none absolute inset-x-0 top-full h-2 bg-linear-to-b from-surface-editor to-transparent" />
    </div>
    <!-- The caption's room plus one gap; the pinned copy above covers it. -->
    <div class="h-[29px] shrink-0" aria-hidden="true" />
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
      <TreeRow
        v-for="(row, index) in rows"
        :id="`${treeId}-${index}`"
        :key="row.path"
        :ref="(el) => bindRow(row.path, el)"
        :row="row"
        :selected="row.path === selected && !selectedUnderStack"
        :menu-open="row.path === menuRow"
        :cursor="focused && index === cursorIndex"
        :highlight="highlightOf(row)"
        :tooltip="tooltip?.(row)"
        @activate="activateRow(row)"
        @contextmenu.stop="emit('menu', row, $event)"
      >
        <template #mark><slot name="mark" :row="row" /></template>
        <template #name><slot name="name" :row="row" :highlight="highlightOf(row)" /></template>
        <template #trailing><slot name="trailing" :row="row" /></template>
      </TreeRow>
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
