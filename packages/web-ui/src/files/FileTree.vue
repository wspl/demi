<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import FileTreeRow from './FileTreeRow.vue'
import { TREE_ROW_PX, stickyTreeRows, type FileTreeRow as Row } from './file-tree'
import { FileBrowserError, type FileBrowserEntry, type FileBrowserFailure, type FileBrowserSource } from './types'
import { baseName, isHiddenName, joinPath, normalizePath, parentPath } from './paths'

/**
 * A directory tree over a `FileBrowserSource`, rooted at `root`. Directories
 * list when first opened and keep their listing; a click on a directory
 * folds or unfolds it, a click on a file asks the host to open it. The
 * selected file's ancestors unfold on their own so it is always in view.
 * The workspace's name heads the tree as a plain caption and stays pinned;
 * under it pin the directories enclosing the selected file, each while its
 * own row has scrolled out above and its contents have not, so the selected
 * file's path stays in sight through its directories and goes past them.
 * A pinned directory scrolls its own row to the top. Loads in flight are
 * dropped when the tree goes away.
 */
const props = defineProps<{
  source: Pick<FileBrowserSource, 'list'>
  root: string
  /** The open file, by absolute path. */
  selected: string | null
}>()

const emit = defineEmits<{
  open: [path: string]
}>()

interface Listing {
  entries: FileBrowserEntry[]
  loading: boolean
  failure: FileBrowserFailure | null
  open: boolean
}

const listings = reactive(new Map<string, Listing>())
const controller = new AbortController()

function listing(path: string): Listing {
  let entry = listings.get(path)
  if (!entry) {
    entry = { entries: [], loading: false, failure: null, open: false }
    listings.set(path, entry)
  }
  return entry
}

function sortEntries(entries: FileBrowserEntry[]): FileBrowserEntry[] {
  return [...entries].sort((a, b) => {
    if (a.isDirectory !== b.isDirectory) {
      return a.isDirectory ? -1 : 1
    }
    const hiddenA = isHiddenName(a.name)
    const hiddenB = isHiddenName(b.name)
    if (hiddenA !== hiddenB) {
      return hiddenA ? 1 : -1
    }
    return a.name.localeCompare(b.name)
  })
}

async function load(path: string): Promise<void> {
  const entry = listing(path)
  if (entry.loading || entry.entries.length > 0) {
    return
  }
  entry.loading = true
  entry.failure = null
  try {
    entry.entries = sortEntries(await props.source.list(path, controller.signal))
  } catch (error) {
    if (controller.signal.aborted) {
      return
    }
    entry.failure = error instanceof FileBrowserError
      ? { kind: error.kind, message: error.message }
      : { kind: 'other', message: error instanceof Error ? error.message : String(error) }
  } finally {
    entry.loading = false
  }
}

function open(path: string): void {
  const entry = listing(path)
  entry.open = true
  void load(path)
}

function toggle(path: string): void {
  const entry = listing(path)
  if (entry.open) {
    entry.open = false
  } else {
    open(path)
  }
}

/** Unfolds every directory from the root down to the selected file. */
function revealSelected(): void {
  const root = normalizePath(props.root)
  const selected = props.selected ? normalizePath(props.selected) : null
  open(root)
  if (!selected || !selected.startsWith(`${root}/`)) {
    return
  }
  let dir = parentPath(selected)
  const ancestors: string[] = []
  while (dir.startsWith(root) && dir !== root) {
    ancestors.push(dir)
    dir = parentPath(dir)
  }
  for (const ancestor of ancestors.reverse()) {
    open(ancestor)
  }
}

watch(() => [props.root, props.selected], revealSelected, { immediate: true })

onBeforeUnmount(() => {
  controller.abort()
})

const rows = computed<Row[]>(() => {
  const out: Row[] = []
  const walk = (dir: string, depth: number, parent: string | null): void => {
    const entry = listings.get(dir)
    if (!entry?.open) {
      return
    }
    for (const item of entry.entries) {
      const path = joinPath(dir, item.name)
      out.push({ path, name: item.name, isDirectory: item.isDirectory, depth, parent })
      if (item.isDirectory) {
        walk(path, depth + 1, path)
      }
    }
  }
  walk(normalizePath(props.root), 0, null)
  return out
})

function rowState(row: Row): { open: boolean; loading: boolean; failure: FileBrowserFailure | null } {
  const entry = row.isDirectory ? listings.get(row.path) : undefined
  return {
    open: entry?.open === true,
    loading: entry?.loading === true,
    failure: entry?.failure ?? null,
  }
}

// The pinned stack: measured from the rows' positions on every scroll and layout.
// Its first slot starts under the viewport padding, the caption and one gap,
// exactly where the first row starts, so a row and its pinned copy coincide.
const STACK_TOP_PX = 4 + TREE_ROW_PX + 1
const scrollArea = ref<InstanceType<typeof ScrollArea> | null>(null)
const rowEls = new Map<string, HTMLElement>()
const stickyPaths = ref<string[]>([])
const stickyOffset = ref(0)
const stickyRows = computed(() => {
  const byPath = new Map(rows.value.map((row) => [row.path, row]))
  return stickyPaths.value.flatMap((path) => {
    const row = byPath.get(path)
    return row ? [row] : []
  })
})

function bindRow(path: string, el: unknown): void {
  if (el instanceof HTMLElement) {
    rowEls.set(path, el)
  } else if (el && typeof el === 'object' && '$el' in el && el.$el instanceof HTMLElement) {
    rowEls.set(path, el.$el)
  } else {
    rowEls.delete(path)
  }
}

function updateSticky(): void {
  const viewport = scrollArea.value?.el
  if (!viewport) {
    return
  }
  const stack = stickyTreeRows(
    rows.value,
    (path) => rowEls.get(path)?.offsetTop,
    viewport.scrollTop,
    STACK_TOP_PX,
    props.selected ? normalizePath(props.selected) : null,
  )
  stickyPaths.value = stack.paths
  stickyOffset.value = stack.offset
}

/** A pinned directory takes the top of the view, under the caption. */
function scrollToRow(path: string): void {
  const viewport = scrollArea.value?.el
  const el = rowEls.get(path)
  if (viewport && el) {
    viewport.scrollTop = el.offsetTop - STACK_TOP_PX
  }
}

/** Moves the tree by `px`, for a host that sets up a scrolled state. */
function scrollBy(px: number): void {
  const viewport = scrollArea.value?.el
  if (viewport) {
    viewport.scrollTop += px
    updateSticky()
  }
}

onMounted(updateSticky)
watch([rows, () => props.selected], () => {
  void nextTick(updateSticky)
})

defineExpose({ scrollToRow, scrollBy })

const rootListing = computed(() => listings.get(normalizePath(props.root)) ?? null)
const rootName = computed(() => baseName(props.root) || '/')

function activate(row: Row): void {
  if (row.isDirectory) {
    toggle(row.path)
  } else {
    emit('open', row.path)
  }
}
</script>

<template>
  <!-- The tree paints its own surface, the editor's, so the pinned stack matches it wherever it sits. -->
  <ScrollArea ref="scrollArea" class="h-full min-h-0 bg-surface-editor" viewport-class="p-1" @scroll="updateSticky">
    <!-- The pinned stack: the caption stays put; the selected file's directories under it
         slide up beneath the caption as the tree scrolls past them. -->
    <!-- Above the rows (whose transformed chevrons would otherwise paint through), below the
         scroll area's thumb; `isolate` keeps the caption's layering inside. The surface covers
         the padding too, so nothing shows through the gaps. -->
    <div class="pointer-events-none absolute inset-x-0 top-0 isolate z-[1] flex flex-col bg-surface-editor p-1 pb-0">
      <div
        class="relative z-10 flex h-7 shrink-0 select-none items-center bg-surface-editor px-2 text-chrome font-medium text-fg-muted"
        :title="root"
      >
        <span class="truncate">{{ rootName }}</span>
      </div>
      <!-- Each pinned row in its own clip; only the deepest slides up as its
           directory leaves, its clip shrinking with it, while the rows above stay. -->
      <div class="flex flex-col gap-px bg-surface-editor pt-px">
        <div
          v-for="(row, index) in stickyRows"
          :key="row.path"
          class="overflow-hidden"
          :style="{ height: `${index === stickyRows.length - 1 ? Math.max(0, TREE_ROW_PX + stickyOffset) : TREE_ROW_PX}px` }"
        >
          <FileTreeRow
            class="pointer-events-auto"
            :style="index === stickyRows.length - 1 ? { transform: `translateY(${stickyOffset}px)` } : undefined"
            :row="row"
            :selected="row.path === selected"
            v-bind="rowState(row)"
            @activate="scrollToRow(row.path)"
          />
        </div>
      </div>
    </div>
    <!-- The caption's room plus one gap; the pinned copy above covers it. -->
    <div class="h-[29px] shrink-0" aria-hidden="true" />
    <div role="tree" :aria-label="rootName" class="flex min-h-full flex-col gap-px">
      <FileTreeRow
        v-for="row in rows"
        :key="row.path"
        :ref="(el) => bindRow(row.path, el)"
        :row="row"
        :selected="row.path === selected"
        v-bind="rowState(row)"
        @activate="activate(row)"
      />
      <div
        v-if="rootListing?.loading && rows.length === 0"
        class="flex flex-1 select-none items-center justify-center py-10 text-fg-subtle"
      >
        <IndeterminateSpinner :size="16" />
      </div>
      <div
        v-else-if="rootListing?.failure && rows.length === 0"
        class="flex flex-1 select-none flex-col items-center justify-center gap-1 px-4 py-10 text-center text-[13px] text-fg-subtle"
      >
        <span>Could not list the workspace.</span>
        <span class="text-[11px] text-fg-faint">{{ rootListing.failure.message }}</span>
      </div>
    </div>
  </ScrollArea>
</template>
