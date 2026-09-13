<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, watch } from 'vue'
import { ChevronRight } from '@lucide/vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from './FileIcon.vue'
import { FileBrowserError, type FileBrowserEntry, type FileBrowserFailure, type FileBrowserSource } from './types'
import { baseName, isHiddenName, joinPath, normalizePath, parentPath } from './paths'

/**
 * A directory tree over a `FileBrowserSource`, rooted at `root`. Directories
 * list when first opened and keep their listing; a click on a directory
 * folds or unfolds it, a click on a file asks the host to open it. The
 * selected file's ancestors unfold on their own so it is always in view.
 * Loads in flight are dropped when the tree goes away.
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

interface Row {
  path: string
  name: string
  isDirectory: boolean
  depth: number
  listing: Listing | null
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
  const walk = (dir: string, depth: number): void => {
    const entry = listings.get(dir)
    if (!entry?.open) {
      return
    }
    for (const item of entry.entries) {
      const path = joinPath(dir, item.name)
      out.push({
        path,
        name: item.name,
        isDirectory: item.isDirectory,
        depth,
        listing: item.isDirectory ? (listings.get(path) ?? null) : null,
      })
      if (item.isDirectory) {
        walk(path, depth + 1)
      }
    }
  }
  walk(normalizePath(props.root), 0)
  return out
})

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
  <ScrollArea class="h-full min-h-0" viewport-class="p-1">
    <div role="tree" :aria-label="rootName" class="flex min-h-full flex-col gap-px">
      <div
        v-for="row in rows"
        :key="row.path"
        role="treeitem"
        :aria-selected="row.path === selected"
        :aria-expanded="row.isDirectory ? row.listing?.open === true : undefined"
        class="flex h-7 shrink-0 cursor-default select-none items-center gap-1 rounded-md pr-1 text-chrome transition-colors duration-200 ease-out"
        :class="row.path === selected ? 'bg-active text-fg-emphasis' : 'text-fg-body hover:bg-hover'"
        :style="{ paddingLeft: `${4 + row.depth * 12}px` }"
        :title="row.name"
        @click="activate(row)"
      >
        <!-- Directories fold on a chevron; files keep its width so names line up. -->
        <span class="flex size-4 shrink-0 items-center justify-center text-fg-faint">
          <IndeterminateSpinner v-if="row.listing?.loading" :size="12" />
          <ChevronRight
            v-else-if="row.isDirectory"
            :size="ICON_PX.in20"
            class="transition-transform duration-150"
            :class="row.listing?.open ? 'rotate-90' : ''"
          />
        </span>
        <FileIcon :name="row.name" :is-directory="row.isDirectory" />
        <span class="truncate">{{ row.name }}</span>
        <span
          v-if="row.listing?.failure"
          class="ml-auto truncate pl-2 text-[11px] text-fg-faint"
          :title="row.listing.failure.message"
          >{{ row.listing.failure.kind === 'permission' ? 'No access' : 'Unavailable' }}</span
        >
      </div>
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
