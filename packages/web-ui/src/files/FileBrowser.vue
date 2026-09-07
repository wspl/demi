<script setup lang="ts">
import { computed, onBeforeUnmount, ref, shallowRef, watch } from 'vue'
import { ArrowLeft, ArrowRight, ArrowUp, ChevronDown, FolderPlus, HardDrive, MapPin, Search } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Button from '../ui/Button.vue'
import Checkbox from '../ui/Checkbox.vue'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import InlineError from '../ui/InlineError.vue'
import Menu from '../ui/Menu.vue'
import MenuGroup from '../ui/MenuGroup.vue'
import MenuItem from '../ui/MenuItem.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import TextInput from '../ui/TextInput.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import SidebarNavItem from '../sidebar/SidebarNavItem.vue'
import FileBrowserAddressBar from './FileBrowserAddressBar.vue'
import FileBrowserList from './FileBrowserList.vue'
import { createFileBrowserHistory, filterEntries, nextSort, sortEntries, type FileBrowserSort, type FileBrowserSortKey } from './file-browser-state'
import { baseName, joinPath, normalizePath, parentPath } from './paths'
import { FileBrowserError, type FileBrowserEntry, type FileBrowserFailure, type FileBrowserHost, type FileBrowserMode, type FileBrowserPlaceGroup, type FileBrowserSource } from './types'

/**
 * The file and folder chooser, laid out like the Windows open dialog: Back, Forward
 * and Up beside the address bar, a filter at the right, places and devices down the
 * left, a detail list, and the name row with the confirm button under it. Below a
 * phone width the rail becomes a Places menu, Forward goes, and the buttons drop
 * under the name.
 *
 * The browser reads through `source` and owns everything else: where it is, what is
 * selected, the history. The caller decides what a chosen path means. Switching a
 * device is the caller's too: it hears `update:hostId` and hands over another source.
 */
const props = withDefaults(defineProps<{
  mode: FileBrowserMode
  source: FileBrowserSource
  /** Where to open; the source's home when absent. */
  initialPath?: string
  places?: FileBrowserPlaceGroup[]
  hosts?: FileBrowserHost[]
  hostId?: string
  /** The confirm button's label: `Open` for a file, `Select Folder` for a folder. */
  confirmLabel?: string
  /** Browsing stays open while choosing is not allowed (a running turn, an archived conversation). */
  confirmDisabled?: boolean
}>(), {
  places: () => [],
  hosts: () => [],
})

const emit = defineEmits<{
  select: [path: string]
  cancel: []
  'update:hostId': [id: string]
}>()

const showHidden = defineModel<boolean>('showHidden', { default: false })

const path = ref(normalizePath(props.initialPath ?? props.source.home))
const history = createFileBrowserHistory(path.value)
const canBack = ref(false)
const canForward = ref(false)
const entries = shallowRef<FileBrowserEntry[]>([])
const loading = ref(false)
const failure = ref<FileBrowserFailure | null>(null)
const query = ref('')
const sort = ref<FileBrowserSort>({ key: 'name', direction: 'asc' })
const selected = ref<string | null>(null)
const name = ref('')
const nameError = ref<string | null>(null)
const creating = ref(false)
const list = ref<InstanceType<typeof FileBrowserList>>()
let pending: AbortController | null = null

const visible = computed(() => sortEntries(filterEntries(entries.value, query.value, showHidden.value), sort.value))
const selectedEntry = computed(() => visible.value.find((entry) => entry.name === selected.value) ?? null)
const currentHost = computed(() => props.hosts.find((host) => host.id === props.hostId))
const hasRail = computed(() => props.places.length > 0 || props.hosts.length > 0)
const confirmLabel = computed(() => props.confirmLabel ?? (props.mode === 'file' ? 'Open' : 'Select Folder'))
const canConfirm = computed(() => {
  if (props.confirmDisabled) return false
  if (selectedEntry.value) return true
  if (name.value.trim()) return true
  return props.mode === 'directory' && !failure.value
})

function toFailure(error: unknown): FileBrowserFailure {
  if (error instanceof FileBrowserError) return { kind: error.kind, message: error.message === error.kind ? undefined : error.message }
  return { kind: 'other', message: error instanceof Error ? error.message : String(error) }
}

async function load(target: string) {
  pending?.abort()
  const controller = new AbortController()
  pending = controller
  loading.value = true
  failure.value = null
  entries.value = []
  try {
    const listed = await props.source.list(target, controller.signal)
    if (controller.signal.aborted) return
    entries.value = listed
  } catch (error) {
    if (controller.signal.aborted) return
    failure.value = toFailure(error)
  } finally {
    if (!controller.signal.aborted) loading.value = false
  }
}

function syncHistory() {
  canBack.value = history.canBack
  canForward.value = history.canForward
}

function show(target: string) {
  path.value = target
  selected.value = null
  name.value = ''
  nameError.value = null
  query.value = ''
  creating.value = false
  void load(target)
}

function goTo(target: string) {
  const next = normalizePath(target)
  history.push(next)
  syncHistory()
  show(next)
}

function back() {
  const previous = history.back()
  syncHistory()
  if (previous !== null) show(previous)
}

function forward() {
  const next = history.forward()
  syncHistory()
  if (next !== null) show(next)
}

function up() {
  if (path.value !== '/') goTo(parentPath(path.value))
}

/** Opens a folder; confirms a file (file mode only, the list enforces it). */
function activate(entry: FileBrowserEntry) {
  if (entry.isDirectory) goTo(joinPath(path.value, entry.name))
  else emit('select', joinPath(path.value, entry.name))
}

/** A typed name or path: `~` is the home, a bare name is inside the current folder. */
function resolveTyped(text: string): string {
  let target = text.trim()
  if (target === '~' || target.startsWith('~/')) target = `${props.source.home}${target.slice(1)}`
  if (!target.startsWith('/')) target = `${path.value}/${target}`
  return normalizePath(target)
}

/**
 * What Enter in the name row and the confirm button do with typed text: a folder is
 * entered, or chosen when the button asks for one; a file is chosen in file mode.
 * The parent's listing says which it is, since the source has no stat.
 */
async function commitTyped(intent: 'enter' | 'confirm') {
  const target = resolveTyped(name.value)
  nameError.value = null
  if (target === '/') {
    goTo('/')
    return
  }
  const parent = parentPath(target)
  const leaf = baseName(target)
  let siblings: FileBrowserEntry[]
  try {
    siblings = await props.source.list(parent)
  } catch (error) {
    nameError.value = toFailure(error).message ?? `${parent} could not be read.`
    return
  }
  const entry = siblings.find((sibling) => sibling.name === leaf)
  if (!entry) {
    nameError.value = props.mode === 'file' ? `There is no ${leaf} in ${parent}.` : `There is no folder ${leaf} in ${parent}.`
    return
  }
  if (entry.isDirectory) {
    if (intent === 'confirm' && props.mode === 'directory') emit('select', target)
    else goTo(target)
    return
  }
  if (props.mode === 'file') emit('select', target)
  else nameError.value = `${leaf} is a file, not a folder.`
}

function confirm() {
  if (selectedEntry.value) {
    if (props.mode === 'file' && selectedEntry.value.isDirectory) goTo(joinPath(path.value, selectedEntry.value.name))
    else emit('select', joinPath(path.value, selectedEntry.value.name))
    return
  }
  if (name.value.trim()) {
    void commitTyped('confirm')
    return
  }
  if (props.mode === 'directory' && !failure.value) emit('select', path.value)
}

function onNameKeydown(event: KeyboardEvent) {
  if (event.key !== 'Enter') return
  event.preventDefault()
  if (selectedEntry.value && name.value.trim() === selectedEntry.value.name) confirm()
  else if (name.value.trim()) void commitTyped('enter')
  else confirm()
}

async function createFolder(folderName: string) {
  const create = props.source.createDirectory
  if (!create) return
  const target = joinPath(path.value, folderName)
  try {
    await create(target)
  } catch (error) {
    creating.value = false
    nameError.value = toFailure(error).message ?? `${folderName} could not be created.`
    return
  }
  creating.value = false
  await load(path.value)
  showHidden.value ||= folderName.startsWith('.')
  selected.value = folderName
  list.value?.focus()
}

// A row chosen in the list fills the name row; typing something else drops the selection.
watch(selected, (next) => {
  if (next !== null) {
    name.value = next
    nameError.value = null
  }
})
watch(name, (next) => {
  if (selected.value !== null && next !== selected.value) selected.value = null
})
watch(query, () => {
  if (selected.value !== null && !visible.value.some((entry) => entry.name === selected.value)) selected.value = null
})

// Another device: the history starts over at its home or the path the caller names.
watch(() => props.source, (source) => {
  const start = normalizePath(props.initialPath ?? source.home)
  history.reset(start)
  syncHistory()
  show(start)
})

void load(path.value)

onBeforeUnmount(() => pending?.abort())

defineExpose({
  /** The directory the browser shows. */
  path,
  goTo,
})
</script>

<template>
  <div class="@container flex min-h-0 flex-1 flex-col">
    <div class="flex shrink-0 items-center gap-2 px-3 py-2">
      <div class="flex shrink-0 items-center">
        <Tooltip content="Back">
          <IconButton :icon="ArrowLeft" variant="ghost" aria-label="Back" :disabled="!canBack" @click="back" />
        </Tooltip>
        <Tooltip content="Forward" class="hidden @md:inline-flex">
          <IconButton :icon="ArrowRight" variant="ghost" aria-label="Forward" :disabled="!canForward" @click="forward" />
        </Tooltip>
        <Tooltip content="Up">
          <IconButton :icon="ArrowUp" variant="ghost" aria-label="Parent folder" :disabled="path === '/'" @click="up" />
        </Tooltip>
      </div>
      <FileBrowserAddressBar
        class="min-w-0 flex-1"
        :path="path"
        :root-label="currentHost?.label"
        :root-icon="currentHost ? (currentHost.icon ?? HardDrive) : undefined"
        @navigate="goTo"
      />
      <TextInput v-model="query" class="hidden w-40 shrink-0 @md:flex" placeholder="Filter" aria-label="Filter this folder">
        <template #prefix>
          <Search :size="ICON_PX.in28" />
        </template>
      </TextInput>
    </div>
    <div class="flex shrink-0 items-center justify-between gap-2 px-3 pb-2">
      <div class="flex items-center gap-2">
        <!-- The rail's places and devices, as a menu where the rail has no room. -->
        <Dropdown v-if="hasRail" :overlay-store="appOverlayStore" class="@md:hidden">
          <template #trigger="{ isOpen }">
            <Button :pressed="isOpen" aria-label="Places">
              <MapPin :size="ICON_PX.in28" />
              <ChevronDown :size="ICON_PX.in24" class="text-fg-subtle" />
            </Button>
          </template>
          <template #content>
            <Menu>
              <MenuGroup v-for="(group, index) in places" :key="group.label ?? index" :label="group.label ?? 'Places'">
                <MenuItem
                  v-for="place in group.places"
                  :key="place.path"
                  :icon="place.icon"
                  :label="place.label ?? (baseName(place.path) || '/')"
                  :title="place.path"
                  @select="goTo(place.path)"
                />
              </MenuGroup>
              <MenuGroup v-if="hosts.length" label="Devices">
                <MenuItem
                  v-for="host in hosts"
                  :key="host.id"
                  :icon="host.icon"
                  :indicator="host.icon ? undefined : host.online ? 'success' : 'muted'"
                  :indicator-label="host.online ? 'Online' : 'Offline'"
                  :label="host.label"
                  choice
                  :is-selected="host.id === hostId"
                  @select="emit('update:hostId', host.id)"
                />
              </MenuGroup>
            </Menu>
          </template>
        </Dropdown>
        <template v-if="source.createDirectory">
          <span class="hidden @md:contents">
            <Button :disabled="!!failure || loading" @click="creating = true">
              <FolderPlus :size="ICON_PX.in28" />
              New folder
            </Button>
          </span>
          <Tooltip content="New folder" class="@md:hidden">
            <IconButton :icon="FolderPlus" aria-label="New folder" :disabled="!!failure || loading" @click="creating = true" />
          </Tooltip>
        </template>
        <!-- The filter sits here where the address bar needs the whole first row. -->
        <TextInput v-model="query" class="w-24 shrink-0 @md:hidden" placeholder="Filter" aria-label="Filter this folder">
          <template #prefix>
            <Search :size="ICON_PX.in28" />
          </template>
        </TextInput>
      </div>
      <Checkbox v-model="showHidden" label="Hidden files" />
    </div>
    <div class="flex min-h-0 flex-1 border-y border-line">
      <ScrollArea
        v-if="hasRail"
        class="hidden w-44 shrink-0 border-r border-line bg-surface @md:block"
        viewport-class="flex flex-col gap-3 p-2"
      >
        <nav v-for="(group, index) in places" :key="group.label ?? index" class="flex flex-col gap-0.5" :aria-label="group.label ?? 'Places'">
          <div v-if="group.label" class="select-none px-2 pb-1 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle">
            {{ group.label }}
          </div>
          <SidebarNavItem
            v-for="place in group.places"
            :key="place.path"
            :icon="place.icon"
            :label="place.label ?? (baseName(place.path) || '/')"
            :pressed="path === normalizePath(place.path)"
            :title="place.path"
            @click="goTo(place.path)"
          />
        </nav>
        <nav v-if="hosts.length" class="flex flex-col gap-0.5" aria-label="Devices">
          <div class="select-none px-2 pb-1 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle">Devices</div>
          <SidebarNavItem
            v-for="host in hosts"
            :key="host.id"
            :icon="host.icon"
            :indicator="host.icon ? undefined : host.online ? 'success' : 'muted'"
            :indicator-label="host.online ? 'Online' : 'Offline'"
            :label="host.label"
            :pressed="host.id === hostId"
            @click="emit('update:hostId', host.id)"
          />
        </nav>
      </ScrollArea>
      <FileBrowserList
        ref="list"
        v-model:selected="selected"
        :entries="visible"
        :mode="mode"
        :sort="sort"
        :loading="loading"
        :failure="failure"
        :query="query"
        :creating="creating"
        @activate="activate"
        @sort="(key: FileBrowserSortKey) => (sort = nextSort(sort, key))"
        @up="up"
        @back="back"
        @forward="forward"
        @create="createFolder"
        @cancel-create="creating = false"
      />
    </div>
    <div class="flex shrink-0 flex-col gap-1.5 px-3 py-3">
      <div class="flex flex-col gap-2 @md:flex-row @md:items-center @md:gap-3">
        <label class="flex min-w-0 flex-1 items-center gap-2">
          <span class="shrink-0 select-none text-chrome text-fg-muted">{{ mode === 'file' ? 'File name:' : 'Folder:' }}</span>
          <TextInput
            v-model="name"
            :placeholder="mode === 'file' ? 'Name or path' : baseName(path) || '/'"
            aria-label="Name"
            spellcheck="false"
            @keydown="onNameKeydown"
          />
        </label>
        <div class="flex shrink-0 items-center justify-end gap-2">
          <Button variant="primary" :disabled="!canConfirm" @click="confirm">{{ confirmLabel }}</Button>
          <Button @click="emit('cancel')">Cancel</Button>
        </div>
      </div>
      <InlineError v-if="nameError" :message="nameError" dismissible @dismiss="nameError = null" />
    </div>
  </div>
</template>
