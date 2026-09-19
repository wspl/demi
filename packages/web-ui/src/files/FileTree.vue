<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, reactive, ref, watch } from 'vue'
import { Download, RefreshCw, Upload } from '@lucide/vue'
import { useContextMenuOwner } from '../composables/useContextMenuOwner'
import { appOverlayStore } from '../overlay/appOverlay'
import CornerDot from '../ui/CornerDot.vue'
import IconButton from '../ui/IconButton.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Popover from '../ui/Popover.vue'
import Tooltip from '../ui/Tooltip.vue'
import Tree from './Tree.vue'
import UploadConflictDialog from './UploadConflictDialog.vue'
import { downloadUrl } from './download'
import { uploadsOf } from './file-uploads'
import type { TreeRow } from './tree'
import { FileBrowserError, type FileBrowserEntry, type FileBrowserFailure, type FileBrowserSource } from './types'
import { baseName, joinPath, normalizePath, parentPath, relativePath } from './paths'
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
 *
 * A right-click offers what the source can do there: a file downloads, and a
 * directory, or the empty space for the workspace itself, takes files
 * uploaded into it. Picked names the directory already has wait on a
 * question, Replace or Skip; the uploads themselves belong to the source
 * (`uploadsOf`), and a directory an upload lands in is listed again.
 */
defineOptions({ inheritAttrs: false })

const props = defineProps<{
  source: Pick<FileBrowserSource, 'list' | 'contents' | 'upload'>
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

// What a right-click offers: a file to download, or a directory to upload into.
interface MenuTarget {
  kind: 'file' | 'directory'
  path: string
}

const menuTarget = ref<MenuTarget | null>(null)
const menu = useContextMenuOwner(() => {
  menuTarget.value = null
})

function openMenu(row: TreeRow | null, event: MouseEvent): void {
  const target: MenuTarget = row === null
    ? { kind: 'directory', path: normalizePath(props.root) }
    : { kind: row.isDirectory ? 'directory' : 'file', path: row.path }
  // With nothing to offer, the browser's own menu stays.
  if (target.kind === 'file' ? !props.source.contents : !props.source.upload)
    return
  menuTarget.value = target
  menu.open(event)
}

function download(path: string): void {
  menu.close()
  if (props.source.contents)
    downloadUrl(props.source.contents.url(path, { download: true }))
}

const uploads = computed(() => uploadsOf(props.source))

// A file that lands in a listed directory shows there.
watch(uploads, (list, _previous, onCleanup) => {
  onCleanup(list.onLanded((directory) => {
    if (listings.get(directory)?.open)
      void load(directory, true)
  }))
}, { immediate: true })

const picker = ref<HTMLInputElement | null>(null)
// The directory the file picker is choosing for, from the menu until it closes.
let pickingFor: string | null = null
const conflict = ref<{ directory: string; files: File[]; taken: string[] } | null>(null)

function chooseFiles(directory: string): void {
  menu.close()
  pickingFor = directory
  picker.value?.click()
}

async function onPicked(): Promise<void> {
  const input = picker.value
  const directory = pickingFor
  const files = [...(input?.files ?? [])]
  pickingFor = null
  // Picking the same file again must fire again.
  if (input)
    input.value = ''
  if (!directory || files.length === 0)
    return
  const taken = await takenNames(directory, files)
  if (taken.length === 0) {
    uploads.value.add(directory, files, new Set())
    return
  }
  conflict.value = { directory, files, taken }
}

/**
 * The picked names the directory already has. A listing that fails asks
 * nothing: the source still refuses to write over a file it was not told to.
 */
async function takenNames(directory: string, files: readonly File[]): Promise<string[]> {
  try {
    const names = new Set((await props.source.list(directory, controller.signal)).map((entry) => entry.name))
    return files.filter((file) => names.has(file.name)).map((file) => file.name)
  } catch {
    return []
  }
}

/** Replace uploads every picked file, writing over the taken names; Skip uploads the others. */
function answerConflict(replace: boolean): void {
  const pending = conflict.value
  conflict.value = null
  if (!pending)
    return
  const taken = new Set(pending.taken)
  const files = replace ? pending.files : pending.files.filter((file) => !taken.has(file.name))
  uploads.value.add(pending.directory, files, replace ? taken : new Set())
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
    v-bind="$attrs"
    :rows="rows"
    :caption="rootName"
    :caption-title="root"
    :selected="selectedPath"
    :tooltip="failureText"
    @activate="activate"
    @menu="openMenu"
  >
    <template #captionTrailing>
      <Tooltip content="Refresh" class="ml-2 shrink-0">
        <IconButton
          :icon="RefreshCw"
          size="xs"
          variant="ghost"
          aria-label="Refresh"
          spin-on-click
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
  <Popover
    :key="menu.menuKey.value"
    :overlay-store="appOverlayStore"
    :is-open="menu.isOpen.value"
    :anchor-x="menu.anchorX.value"
    :anchor-y="menu.anchorY.value"
    :anchor-context-el="menu.anchorContextEl.value"
    :offset="0"
    @close="menu.close()"
  >
    <Menu>
      <MenuItem v-if="menuTarget?.kind === 'file'" :icon="Download" label="Download" @select="download(menuTarget.path)" />
      <MenuItem v-else-if="menuTarget" :icon="Upload" label="Upload files…" @select="chooseFiles(menuTarget.path)" />
    </Menu>
  </Popover>
  <input ref="picker" type="file" multiple class="hidden" @change="onPicked">
  <UploadConflictDialog
    :is-open="conflict !== null"
    :overlay-store="appOverlayStore"
    :names="conflict?.taken ?? []"
    :directory="conflict ? relativePath(root, conflict.directory) || rootName : ''"
    :others="conflict ? conflict.files.length - conflict.taken.length : 0"
    @replace="answerConflict(true)"
    @skip="answerConflict(false)"
    @cancel="conflict = null"
  />
</template>
