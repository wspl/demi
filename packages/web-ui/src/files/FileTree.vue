<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, reactive, ref, watch } from 'vue'
import { RefreshCw } from '@lucide/vue'
import CornerDot from '../ui/CornerDot.vue'
import IconButton from '../ui/IconButton.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import Tooltip from '../ui/Tooltip.vue'
import Tree from './Tree.vue'
import type { TreeRow } from './tree'
import { FileBrowserError, type FileBrowserEntry, type FileBrowserFailure, type FileBrowserSource } from './types'
import { baseName, joinPath, normalizePath, parentPath } from './paths'
import { DEFAULT_SORT, sortEntries } from './file-browser-state'

/**
 * A directory tree over a `FileBrowserSource`, rooted at `root`, on a
 * `Tree`. Directories list when first opened and keep their listing; a
 * click on a directory folds or unfolds it, a click on a file asks the host
 * to open it. The selected row's ancestors unfold on their own so it is
 * always in view, and a row the host reveals is scrolled to as well, once
 * its directory's listing has put it in. A directory being listed spins at
 * its row's end; one that could not be listed wears a red dot on its icon
 * and tells why on hover.
 * The control at the caption's end lists every open directory again, turning
 * while they load. Loads in flight are dropped when the tree goes away.
 */
const props = defineProps<{
  source: Pick<FileBrowserSource, 'list'>
  root: string
  /** What heads the tree in place of the root directory's name. */
  rootName?: string
  /** The selected row, by absolute path: the open file, or a directory the host located. */
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

/** Lists a directory once; `again` lists it anew, its rows staying until the new ones land. */
async function load(path: string, again = false): Promise<void> {
  const entry = listing(path)
  if (entry.loading || (entry.entries.length > 0 && !again)) {
    return
  }
  entry.loading = true
  entry.failure = null
  try {
    // A tree has no sort controls: folders first, then names.
    entry.entries = sortEntries(await props.source.list(path, controller.signal), DEFAULT_SORT)
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

const refreshing = ref(false)

/** Lists every open directory again, the root included; the folds and rows stay until the new listings land. */
async function refresh(): Promise<void> {
  if (refreshing.value) {
    return
  }
  refreshing.value = true
  const reloads: Promise<void>[] = []
  for (const [path, entry] of listings) {
    if (entry.open) {
      reloads.push(load(path, true))
    }
  }
  try {
    await Promise.all(reloads)
  } finally {
    refreshing.value = false
  }
}

function toggle(path: string): void {
  const entry = listing(path)
  if (entry.open) {
    entry.open = false
  } else {
    open(path)
  }
}

/** Unfolds the root and every directory from it down to `path`'s own directory. */
function unfoldTo(path: string | null): void {
  const root = normalizePath(props.root)
  const target = path ? normalizePath(path) : null
  open(root)
  if (!target || !target.startsWith(`${root}/`)) {
    return
  }
  let dir = parentPath(target)
  const ancestors: string[] = []
  while (dir.startsWith(root) && dir !== root) {
    ancestors.push(dir)
    dir = parentPath(dir)
  }
  for (const ancestor of ancestors.reverse()) {
    open(ancestor)
  }
}

watch(() => [props.root, props.selected], () => unfoldTo(props.selected), { immediate: true })

// A row the host asked to see: scrolled to once the listings unfolding to it have put it in.
const revealing = ref<string | null>(null)

function reveal(path: string): void {
  unfoldTo(path)
  const target = normalizePath(path)
  // The root heads the tree as its caption; it has no row to scroll to.
  revealing.value = target === normalizePath(props.root) ? null : target
}

onBeforeUnmount(() => {
  controller.abort()
})

const rows = computed<TreeRow[]>(() => {
  const out: TreeRow[] = []
  const walk = (dir: string, depth: number, parent: string | null): void => {
    const entry = listings.get(dir)
    if (!entry?.open) {
      return
    }
    for (const item of entry.entries) {
      const path = joinPath(dir, item.name)
      const open = item.isDirectory && listings.get(path)?.open === true
      out.push({ path, name: item.name, isDirectory: item.isDirectory, depth, parent, open })
      if (open) {
        walk(path, depth + 1, path)
      }
    }
  }
  walk(normalizePath(props.root), 0, null)
  return out
})

const selectedPath = computed(() => (props.selected ? normalizePath(props.selected) : null))
const rootListing = computed(() => listings.get(normalizePath(props.root)) ?? null)
const rootName = computed(() => props.rootName ?? (baseName(props.root) || '/'))

function failureOf(row: TreeRow): FileBrowserFailure | null {
  return row.isDirectory ? (listings.get(row.path)?.failure ?? null) : null
}

function isLoading(row: TreeRow): boolean {
  return row.isDirectory && listings.get(row.path)?.loading === true
}

/** Why the directory could not be listed, for the whole row's tooltip and its dot. */
function failureText(row: TreeRow): string {
  const failure = failureOf(row)
  if (!failure) {
    return ''
  }
  const heading = failure.kind === 'permission' ? 'No access' : 'Unavailable'
  return failure.message ? `${heading}: ${failure.message}` : heading
}

function activate(row: TreeRow): void {
  if (row.isDirectory) {
    toggle(row.path)
  } else {
    emit('open', row.path)
  }
}

// The generic `Tree` has no instance type to name; its exposed surface is spelled out.
const tree = ref<{ scrollToRow(path: string): void; revealRow(path: string): void; scrollBy(px: number): void } | null>(null)

watch([rows, revealing], () => {
  const target = revealing.value
  if (target === null || !rows.value.some((row) => row.path === target)) {
    return
  }
  revealing.value = null
  void nextTick(() => tree.value?.revealRow(target))
})

defineExpose({
  reveal,
  scrollToRow: (path: string) => tree.value?.scrollToRow(path),
  scrollBy: (px: number) => tree.value?.scrollBy(px),
})
</script>

<template>
  <Tree
    ref="tree"
    :rows="rows"
    :caption="rootName"
    :caption-title="root"
    :selected="selectedPath"
    :tooltip="failureText"
    @activate="activate"
  >
    <template #captionTrailing>
      <Tooltip content="Refresh" class="ml-2 shrink-0">
        <IconButton
          :icon="RefreshCw"
          size="xs"
          variant="ghost"
          aria-label="Refresh"
          :spinning="refreshing"
          @click="refresh"
        />
      </Tooltip>
    </template>
    <template #mark="{ row }">
      <CornerDot v-if="failureOf(row)" tone="danger" size="xs" ring="editor" :label="failureText(row)" />
    </template>
    <template #trailing="{ row }">
      <IndeterminateSpinner
        v-if="isLoading(row)"
        class="ml-auto shrink-0 text-fg-faint"
        :size="12"
        :stroke-width="1.5"
      />
    </template>
    <template #empty>
      <div
        v-if="rootListing?.loading"
        class="flex flex-1 select-none items-center justify-center py-10 text-fg-subtle"
      >
        <IndeterminateSpinner :size="16" />
      </div>
      <div
        v-else-if="rootListing?.failure"
        class="flex flex-1 select-none flex-col items-center justify-center gap-1 px-4 py-10 text-center text-[13px] text-fg-subtle"
      >
        <span>Could not list the workspace.</span>
        <span class="text-[11px] text-fg-faint">{{ rootListing.failure.message }}</span>
      </div>
    </template>
  </Tree>
</template>
