<script setup lang="ts">
import { computed, onBeforeUnmount, ref, shallowRef, watch } from 'vue'
import { ArrowLeft, ArrowRight, ArrowUp, Eye, EyeOff, File, Folder, FolderPlus, MapPin, Monitor } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Button from '../ui/Button.vue'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import MenuGroup from '../ui/MenuGroup.vue'
import MenuItem from '../ui/MenuItem.vue'
import ScrollArea from '../ui/ScrollArea.vue'
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
 * and Up, then the device and beside it the path from that device's root; places
 * down the left; a detail list; and a status row with the confirm button. The
 * status row's left holds New folder and the hidden-files switch as icons, and says
 * what is selected or what can be. Below a phone width the rail becomes a Places
 * menu there too, and Forward goes.
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
  /** The devices the address bar's picker offers; none means no picker. */
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
const sort = ref<FileBrowserSort>({ key: 'name', direction: 'asc' })
const selected = ref<string | null>(null)
const error = ref<string | null>(null)
const creating = ref(false)
const list = ref<InstanceType<typeof FileBrowserList>>()
let pending: AbortController | null = null

const visible = computed(() => sortEntries(filterEntries(entries.value, '', showHidden.value), sort.value))
const selectedEntry = computed(() => visible.value.find((entry) => entry.name === selected.value) ?? null)
const currentHost = computed(() => props.hosts.find((host) => host.id === props.hostId))
const hasRail = computed(() => props.places.length > 0)
const confirmLabel = computed(() => props.confirmLabel ?? (props.mode === 'file' ? 'Open' : 'Select Folder'))
const canConfirm = computed(() => {
  if (props.confirmDisabled) return false
  if (selectedEntry.value) return true
  return props.mode === 'directory' && !failure.value
})
/** The status row: what is selected, or what can be. */
const status = computed(() => {
  const entry = selectedEntry.value
  if (entry) return { icon: entry.isDirectory ? Folder : File, text: entry.name, muted: false }
  if (props.mode === 'file') return { icon: null, text: 'Select a file', muted: true }
  return { icon: null, text: `Select a folder, or use ${baseName(path.value) || '/'}`, muted: true }
})

function toFailure(err: unknown): FileBrowserFailure {
  if (err instanceof FileBrowserError) return { kind: err.kind, message: err.message === err.kind ? undefined : err.message }
  return { kind: 'other', message: err instanceof Error ? err.message : String(err) }
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
  } catch (err) {
    if (controller.signal.aborted) return
    failure.value = toFailure(err)
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
  error.value = null
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

function confirm() {
  const entry = selectedEntry.value
  if (entry) {
    if (props.mode === 'file' && entry.isDirectory) goTo(joinPath(path.value, entry.name))
    else emit('select', joinPath(path.value, entry.name))
    return
  }
  if (props.mode === 'directory' && !failure.value) emit('select', path.value)
}

async function createFolder(folderName: string) {
  const create = props.source.createDirectory
  if (!create) return
  const target = joinPath(path.value, folderName)
  try {
    await create(target)
  } catch (err) {
    creating.value = false
    error.value = toFailure(err).message ?? `${folderName} could not be created.`
    return
  }
  creating.value = false
  await load(path.value)
  showHidden.value ||= folderName.startsWith('.')
  selected.value = folderName
  list.value?.focus()
}

watch(selected, (next) => {
  if (next !== null) error.value = null
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
  <div class="@container flex min-h-0 flex-1 select-none flex-col">
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
      <!-- The device, its own control; the path beside it starts at that device's root. -->
      <Dropdown v-if="hosts.length" :overlay-store="appOverlayStore" variant="default" trigger-label="Device" class="shrink-0">
        <template #trigger>
          <component :is="currentHost?.icon ?? Monitor" :size="ICON_PX.in28" class="text-fg-muted" />
          <span class="max-w-32 truncate">{{ currentHost?.label ?? 'Device' }}</span>
        </template>
        <template #content>
          <Menu>
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
          </Menu>
        </template>
      </Dropdown>
      <FileBrowserAddressBar class="min-w-0 flex-1" :path="path" @navigate="goTo" />
    </div>
    <div class="flex min-h-0 flex-1 border-y border-line">
      <ScrollArea
        v-if="hasRail"
        class="hidden w-44 shrink-0 border-r border-line bg-surface @md:block"
        viewport-class="flex flex-col gap-3 p-2"
      >
        <nav v-for="(group, index) in places" :key="group.label ?? index" class="flex flex-col gap-0.5" :aria-label="group.label ?? 'Places'">
          <div v-if="group.label" class="px-2 pb-1 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle">
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
      </ScrollArea>
      <FileBrowserList
        ref="list"
        v-model:selected="selected"
        :entries="visible"
        :mode="mode"
        :sort="sort"
        :loading="loading"
        :failure="failure"
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
    <div class="flex shrink-0 flex-col gap-2 px-3 py-2 @md:flex-row @md:items-center @md:gap-3">
      <div class="flex min-w-0 flex-1 items-center gap-3">
      <div class="flex shrink-0 items-center gap-0.5">
        <!-- The rail's places, as a menu where the rail has no room. -->
        <Dropdown v-if="hasRail" :overlay-store="appOverlayStore" class="@md:hidden">
          <template #trigger="{ isOpen }">
            <Tooltip content="Places">
              <IconButton :icon="MapPin" variant="ghost" :pressed="isOpen" aria-label="Places" />
            </Tooltip>
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
            </Menu>
          </template>
        </Dropdown>
        <Tooltip v-if="source.createDirectory" content="New folder">
          <IconButton :icon="FolderPlus" variant="ghost" aria-label="New folder" :disabled="!!failure || loading" @click="creating = true" />
        </Tooltip>
        <Tooltip :content="showHidden ? 'Hide hidden files' : 'Show hidden files'">
          <IconButton :icon="showHidden ? EyeOff : Eye" variant="ghost" :pressed="showHidden" :aria-label="showHidden ? 'Hide hidden files' : 'Show hidden files'" @click="showHidden = !showHidden" />
        </Tooltip>
      </div>
      <div class="flex min-w-0 flex-1 items-center gap-2 text-chrome" role="status">
        <template v-if="error">
          <span class="truncate text-on-danger" :title="error">{{ error }}</span>
        </template>
        <template v-else>
          <component :is="status.icon" v-if="status.icon" :size="ICON_PX.in28" class="shrink-0 text-fg-muted" />
          <span class="truncate" :class="status.muted ? 'text-fg-subtle' : 'text-fg'" :title="status.text">{{ status.text }}</span>
        </template>
      </div>
      </div>
      <div class="flex shrink-0 items-center justify-end gap-2">
        <Button variant="primary" :disabled="!canConfirm" @click="confirm">{{ confirmLabel }}</Button>
        <Button @click="emit('cancel')">Cancel</Button>
      </div>
    </div>
  </div>
</template>
