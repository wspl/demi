<script setup lang="ts" generic="R extends Row">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import DropOutline from '../ui/DropOutline.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import TreeRow from './TreeRow.vue'
import { TREE_ROW_PITCH_PX, TREE_ROW_PX, stickyTreeRows, treeBlock, type TreeDropTarget, type TreeRow as Row } from './tree'

/**
 * A tree of rows laid out by its host: a caption, then the rows, on the
 * editor's surface. A click on a row asks the host to activate it (fold a
 * directory, open a file). The caption stays pinned; under it pin the
 * directories the rows at the top sit in, each while its own row has
 * scrolled out above and its contents have not, so the way to what is in
 * view stays in sight and goes once the tree is past it. A pinned
 * directory scrolls its own row to the top, or acts as the row once it is
 * there. The rows are dressed through
 * the slots `mark`, `name` and `trailing`, given the row; `tooltip` covers a
 * row with a hint; `empty` fills the tree while it has no rows. A right-click
 * asks the host for a menu (`menu`), on a row or, given no row, on the tree's
 * empty space; a host with nothing to offer leaves the browser's own.
 *
 * Where a drag would drop (`dropTarget`) lights under a dashed line: a
 * directory row with the rows it holds, its pinned copies too, or the whole
 * tree. The host follows the drag itself and asks which row an event
 * happened in (`rowAt`).
 */
const props = defineProps<{
  rows: readonly R[]
  /** The tree's name, heading it as a plain caption. */
  caption: string
  /** What the caption stands for, on hover. */
  captionTitle?: string
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
  name?(props: { row: R }): unknown
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

/** A pinned directory takes the top of the view, under the caption. */
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
  // A row's ancestors are pinned above it once it reaches the top.
  const belowAncestors = el.offsetTop - STACK_TOP_PX - row.depth * TREE_ROW_PITCH_PX
  const atBottom = el.offsetTop + el.offsetHeight - viewport.clientHeight
  if (viewport.scrollTop > belowAncestors) {
    viewport.scrollTop = belowAncestors
  } else if (viewport.scrollTop < atBottom) {
    viewport.scrollTop = atBottom
  }
}

/**
 * A click on a pinned row brings its directory to the top; one already there
 * (its pinned copy lying over the row itself) acts as the row would.
 */
function activatePinned(row: R): void {
  const viewport = scrollArea.value?.el
  const el = rowEls.get(row.path)
  if (viewport && el && viewport.scrollTop === el.offsetTop - STACK_TOP_PX) {
    emit('activate', row)
    return
  }
  scrollToRow(row.path)
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
            <template #name><slot name="name" :row="row" /></template>
            <template #trailing><slot name="trailing" :row="row" /></template>
          </TreeRow>
          <div v-if="dropPaths.has(row.path)" class="pointer-events-none absolute inset-0 bg-tint-accent/50" />
        </div>
      </div>
      <!-- Only scrolled rows need a fall-off below the stack that covers them. -->
      <div v-if="scrolled && rows.length > 0" class="pointer-events-none absolute inset-x-0 top-full h-2 bg-linear-to-b from-surface-editor to-transparent" />
    </div>
    <!-- The caption's room plus one gap; the pinned copy above covers it. -->
    <div class="h-[29px] shrink-0" aria-hidden="true" />
    <div role="tree" :aria-label="caption" class="flex min-h-full flex-col gap-px">
      <TreeRow
        v-for="row in rows"
        :key="row.path"
        :ref="(el) => bindRow(row.path, el)"
        :row="row"
        :selected="row.path === selected && !selectedUnderStack"
        :menu-open="row.path === menuRow"
        :tooltip="tooltip?.(row)"
        @activate="emit('activate', row)"
        @contextmenu.stop="emit('menu', row, $event)"
      >
        <template #mark><slot name="mark" :row="row" /></template>
        <template #name><slot name="name" :row="row" /></template>
        <template #trailing><slot name="trailing" :row="row" /></template>
      </TreeRow>
      <slot v-if="rows.length === 0" name="empty" />
      <slot v-else name="after" />
    </div>
    <!-- Over the listed rows and under the stack, so a block whose row has scrolled
         under it runs on beneath the pinned copies. -->
    <div
      v-if="dropBox"
      class="pointer-events-none absolute inset-x-1 rounded-md bg-tint-accent/50"
      :style="{ top: `${dropBox.top}px`, height: `${dropBox.height}px` }"
    >
      <DropOutline radius="6px" />
    </div>
    <div v-if="dropTarget?.kind === 'tree'" class="pointer-events-none absolute inset-1 z-[1] rounded-md bg-tint-accent/50">
      <DropOutline radius="6px" />
    </div>
  </ScrollArea>
</template>
