<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, reactive, ref, watch } from 'vue'
import { Download, RefreshCw, Upload } from '@lucide/vue'
import { useElementSize } from '@vueuse/core'
import { useContextMenuOwner } from '../composables/useContextMenuOwner'
import { useFileDrop } from '../composables/useFileDrop'
import { showToast } from '../infra/toast'
import { appOverlayStore } from '../overlay/appOverlay'
import CornerDot from '../ui/CornerDot.vue'
import IconButton from '../ui/IconButton.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Popover from '../ui/Popover.vue'
import ResizeHandle from '../ui/ResizeHandle.vue'
import Tooltip from '../ui/Tooltip.vue'
import FileUploadList from './FileUploadList.vue'
import Tree from './Tree.vue'
import UploadConflictDialog from './UploadConflictDialog.vue'
import { downloadUrl } from './download'
import { droppedEntries, droppedItems, type DroppedEntry } from './dropped'
import {
  clashPlacement,
  uploadItemName,
  uploadsOf,
  type ClashAnswer,
  type UploadClash,
  type UploadItem,
} from './file-uploads'
import type { TreeDropTarget, TreeRow } from './tree'
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
 * uploaded into it. Files and folders dragged in from the desktop upload
 * where they drop: into the directory under the pointer, the directory of a
 * file under it, or the workspace over the empty space; that place lights
 * while the drag is over it, and a closed directory the drag rests on opens.
 * Names the directory already has wait on a question: Replace or Skip, and
 * Merge once a folder meets a folder. The uploads themselves belong to the
 * source (`uploadsOf`), and a directory an upload changes is listed again.
 *
 * While the source has uploads they list under the tree, fitting their rows
 * up to `UPLOADS_FIT_PX`; the divider between the two sizes the list, which
 * then keeps that height, leaving the tree at least `TREE_MIN_PX`.
 */
defineOptions({ inheritAttrs: false })

const props = defineProps<{
  source: Pick<FileBrowserSource, 'list' | 'contents' | 'upload' | 'createDirectory' | 'remove'>
  root: string
  /** What heads the tree in place of the root directory's name. */
  rootName?: string
  /** The selected row, by absolute path: the open file, or a directory the host located. */
  selected: string | null
  /**
   * A drag of files shown held over this path as if one were, a row's or the
   * root's for the empty space: for a specimen of the drop state.
   */
  dropping?: string
}>()

const emit = defineEmits<{
  open: [path: string]
}>()

/** The uploads list's height before its divider is moved: its rows, up to this. */
const UPLOADS_FIT_PX = 240
/** The least the divider leaves the uploads list: its caption and one row. */
const UPLOADS_MIN_PX = 76
/** The least the divider leaves the tree: its caption and two rows. */
const TREE_MIN_PX = 96

// The generic `Tree` has no instance type to name; its exposed surface is spelled out.
const tree = ref<{
  $el: HTMLElement
  scrollToRow(path: string): void
  revealRow(path: string): void
  scrollBy(px: number): void
  rowAt(target: EventTarget | null): TreeRow | null
} | null>(null)

interface Listing {
  entries: FileBrowserEntry[]
  loading: boolean
  /** Listed again once the listing on its way is in: a change may have come too late for it. */
  again: boolean
  failure: FileBrowserFailure | null
  open: boolean
}

const listings = reactive(new Map<string, Listing>())
const controller = new AbortController()

function listing(path: string): Listing {
  let entry = listings.get(path)
  if (!entry) {
    entry = { entries: [], loading: false, again: false, failure: null, open: false }
    listings.set(path, entry)
  }
  return entry
}

/**
 * Lists a directory once; `again` lists it anew, its rows staying until the
 * new ones land, and once more after a listing already on its way.
 */
async function load(path: string, again = false): Promise<void> {
  const entry = listing(path)
  if (entry.loading) {
    entry.again ||= again
    return
  }
  if (entry.entries.length > 0 && !again) {
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
  if (entry.again && !controller.signal.aborted) {
    entry.again = false
    void load(path, true)
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

// The uploads list's height, once its divider has set one; until then it fits its rows.
const uploadsHeight = ref<number | null>(null)
const panel = ref<HTMLElement | null>(null)
const uploadsList = ref<InstanceType<typeof FileUploadList> | null>(null)
const { height: panelHeight } = useElementSize(panel)
const { height: listHeight } = useElementSize(uploadsList, undefined, { box: 'border-box' })
const uploadsRoom = computed(() => Math.max(UPLOADS_MIN_PX, Math.floor(panelHeight.value - TREE_MIN_PX)))
const uploadsSize = computed({
  get: () => uploadsHeight.value ?? Math.round(listHeight.value),
  set: (height: number) => {
    uploadsHeight.value = height
  },
})

// A directory an upload changes shows it, when it is listed.
watch(uploads, (list, _previous, onCleanup) => {
  onCleanup(list.onChanged((directory) => {
    if (listings.get(directory)?.open)
      void load(directory, true)
  }))
}, { immediate: true })

const picker = ref<HTMLInputElement | null>(null)
// The directory the file picker is choosing for, from the menu until it closes.
let pickingFor: string | null = null
// Items waiting on the question their names ask, with the directory they go into.
const question = ref<{ directory: string; items: UploadItem[]; clashes: UploadClash[] } | null>(null)

function chooseFiles(directory: string): void {
  menu.close()
  pickingFor = directory
  picker.value?.click()
}

function onPicked(): void {
  const input = picker.value
  const directory = pickingFor
  const files = [...(input?.files ?? [])]
  pickingFor = null
  // Picking the same file again must fire again.
  if (input)
    input.value = ''
  if (directory)
    void offer(directory, files.map((file) => ({ kind: 'file', file })))
}

/** Uploads `items` into `directory`, asking first about the names it has. */
async function offer(directory: string, items: readonly UploadItem[]): Promise<void> {
  if (items.length === 0)
    return
  const clashes = await clashesIn(directory, items)
  if (clashes.length === 0) {
    uploads.value.add(directory, items.map((item) => ({ item, placement: 'add' })))
    return
  }
  question.value = { directory, items: [...items], clashes }
}

/**
 * The items whose names the directory has, with what is there. A listing
 * that fails asks nothing: the source still refuses to write over a file it
 * was not told to.
 */
async function clashesIn(directory: string, items: readonly UploadItem[]): Promise<UploadClash[]> {
  let entries: FileBrowserEntry[]
  try {
    entries = await props.source.list(directory, controller.signal)
  } catch {
    return []
  }
  const taken = new Map(entries.map((entry) => [entry.name, entry.isDirectory]))
  return items.flatMap((item) => {
    const name = uploadItemName(item)
    const takenByDirectory = taken.get(name)
    return takenByDirectory === undefined ? [] : [{ name, isDirectory: item.kind === 'folder', takenByDirectory }]
  })
}

/** Each item goes in as the answer places it; one whose name is free is added. */
function answer(choice: ClashAnswer): void {
  const pending = question.value
  question.value = null
  if (!pending)
    return
  const clashes = new Map(pending.clashes.map((clash) => [clash.name, clash]))
  const placed = pending.items.flatMap((item) => {
    const clash = clashes.get(uploadItemName(item))
    const placement = clash ? clashPlacement(choice, clash.isDirectory, clash.takenByDirectory) : 'add'
    return placement ? [{ item, placement }] : []
  })
  uploads.value.add(pending.directory, placed)
}

/** What else the question's items hold, which the directory does not have. */
const questionOthers = computed(() => {
  const pending = question.value
  const clashing = new Set(pending?.clashes.map((clash) => clash.name))
  const rest = pending?.items.filter((item) => !clashing.has(uploadItemName(item))) ?? []
  const folders = rest.filter((item) => item.kind === 'folder').length
  return { files: rest.length - folders, folders }
})

/** How long a drag rests on a closed directory before it opens, to drop deeper. */
const OPEN_ON_HOLD_MS = 600

// The row a drag of files is over, by path, or the root's over the empty space.
const dragOver = ref<string | null>(null)
// Opens the closed directory a drag rests on; stopped when the drag moves on, leaves or drops.
let holdTimer: ReturnType<typeof setTimeout> | undefined

onBeforeUnmount(() => {
  clearTimeout(holdTimer)
})

/** Where a drop over `path` goes: a directory takes it, a file's directory does, and the root the empty space's. */
function dropDirectory(path: string): string {
  const root = normalizePath(props.root)
  const row = rows.value.find((entry) => entry.path === path)
  if (!row)
    return root
  return row.isDirectory ? row.path : (row.parent ?? root)
}

const dropTarget = computed<TreeDropTarget | null>(() => {
  const over = dragOver.value ?? props.dropping
  if (over === undefined)
    return null
  const directory = dropDirectory(over)
  return directory === normalizePath(props.root) ? { kind: 'tree' } : { kind: 'row', path: directory }
})

useFileDrop(() => tree.value?.$el, {
  enabled: () => props.source.upload !== undefined,
  over(event) {
    const row = tree.value?.rowAt(event.target) ?? null
    const path = row?.path ?? normalizePath(props.root)
    if (dragOver.value === path)
      return
    dragOver.value = path
    clearTimeout(holdTimer)
    if (row?.isDirectory && !row.open)
      holdTimer = setTimeout(() => open(row.path), OPEN_ON_HOLD_MS)
  },
  leave() {
    clearTimeout(holdTimer)
    dragOver.value = null
  },
  drop(event) {
    const row = tree.value?.rowAt(event.target) ?? null
    const directory = dropDirectory(row?.path ?? normalizePath(props.root))
    // The transfer empties once this handler returns; its entries stay readable.
    const entries = event.dataTransfer ? droppedEntries(event.dataTransfer) : []
    void receive(directory, entries)
  },
})

/** Reads what a drop carried and offers it to `directory`; a drop that cannot be read says why. */
async function receive(directory: string, entries: readonly (DroppedEntry | File)[]): Promise<void> {
  let items: UploadItem[]
  try {
    items = await droppedItems(entries)
  } catch (error) {
    showToast({
      title: 'Could not read what was dropped',
      message: error instanceof Error ? error.message : String(error),
      tone: 'danger',
    })
    return
  }
  await offer(directory, items)
}

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
  /** Uploads items into a directory as a drop there would, asking first about the names it has. */
  upload: offer,
})
</script>

<template>
  <div ref="panel" class="flex h-full min-h-0 flex-col" v-bind="$attrs">
    <Tree
      ref="tree"
      class="min-h-0 flex-1"
      :rows="rows"
      :caption="rootName"
      :caption-title="root"
      :selected="selectedPath"
      :menu-row="menu.isOpen.value ? menuTarget?.path : null"
      :drop-target="dropTarget"
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
    <template v-if="uploads.items.length > 0">
      <ResizeHandle
        v-model="uploadsSize"
        orientation="horizontal"
        side="end"
        :min="UPLOADS_MIN_PX"
        :max="uploadsRoom"
        label="uploads"
      />
      <FileUploadList
        ref="uploadsList"
        class="shrink-0"
        :style="uploadsHeight === null ? { maxHeight: `${UPLOADS_FIT_PX}px` } : { height: `${uploadsHeight}px` }"
        :uploads="uploads"
      />
    </template>
  </div>
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
    :is-open="question !== null"
    :overlay-store="appOverlayStore"
    :clashes="question?.clashes ?? []"
    :directory="question ? relativePath(root, question.directory) || rootName : ''"
    :others="questionOthers"
    @replace="answer('replace')"
    @merge="answer('merge')"
    @skip="answer('skip')"
    @cancel="question = null"
  />
</template>
