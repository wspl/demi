<script setup lang="ts">
import {
  computed,
  h,
  onBeforeUnmount,
  ref,
  shallowRef,
  watch,
  type Component,
} from 'vue'
import {
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  ChevronDown,
  Eye,
  EyeOff,
  FolderPlus,
  MapPin,
} from '@lucide/vue'
import { CLOUD_HOST_ID, hostIcon } from '../hosts/icons'
import { appOverlayStore } from '../overlay/appOverlay'
import Button from '../ui/Button.vue'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import InlineError from '../ui/InlineError.vue'
import Menu from '../ui/Menu.vue'
import MenuGroup from '../ui/MenuGroup.vue'
import MenuItem from '../ui/MenuItem.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import SidebarNavItem from '../sidebar/SidebarNavItem.vue'
import FileBrowserAddressBar from './FileBrowserAddressBar.vue'
import FileBrowserList from './FileBrowserList.vue'
import FileIcon from './FileIcon.vue'
import { landmarkIcon } from './file-icons'
import {
  createFileBrowserHistory,
  filterEntries,
  nextSort,
  sortEntries,
  type FileBrowserSort,
  type FileBrowserSortKey,
} from './file-browser-state'
import { baseName, joinPath, normalizePath, parentPath } from './paths'
import {
  FileBrowserError,
  type FileBrowserEntry,
  type FileBrowserFailure,
  type FileBrowserHost,
  type FileBrowserMode,
  type FileBrowserPlace,
  type FileBrowserPlaceGroup,
  type FileBrowserSource,
} from './types'

/**
 * The file and folder chooser, laid out like the Windows open dialog: Back, Forward
 * and Up, then the device and beside it the path from that device's root; places
 * root, with New folder and the hidden-files switch as icons after it; places down
 * the left; a detail list; and a status row that says what is selected or what can
 * be, with the confirm button. Below a phone width the rail becomes a Places menu
 * at the start of the status row, and Forward goes.
 *
 * The browser reads through `source` and owns everything else: where it is, what is
 * selected, the history. The caller decides what a chosen path means. Switching a
 * device is the caller's too: it hears `update:hostId` and hands over another source.
 */
const props = withDefaults(
  defineProps<{
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
    confirmPending?: boolean
  }>(),
  {
    places: () => [],
    hosts: () => [],
  },
)

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
const sort = ref<FileBrowserSort>({
  key: null,
  direction: 'asc',
})
const selected = ref<string | null>(null)
const error = ref<string | null>(null)
const folderCreation = ref<'idle' | 'naming' | 'saving'>('idle')
const list = ref<InstanceType<typeof FileBrowserList>>()
let pending: AbortController | null = null

/** The rows in view, each folder carrying the glyph its place earns (the home under `/Users`). */
const visible = computed(() =>
  sortEntries(
    filterEntries(entries.value, '', showHidden.value),
    sort.value,
  ).map((entry) =>
    entry.isDirectory
      ? {
          ...entry,
          icon: landmarkIcon(joinPath(path.value, entry.name), props.source),
        }
      : entry,
  ),
)
const selectedEntry = computed(
  () => visible.value.find((entry) => entry.name === selected.value) ?? null,
)
const currentHost = computed(() =>
  props.hosts.find((host) => host.id === props.hostId),
)
const hasRail = computed(() => props.places.length > 0)
/** A place's glyph: the theme folder its name resolves to, or the id the caller names. */
function placeIcon(place: FileBrowserPlace): Component {
  const target = normalizePath(place.path)
  return () =>
    h(FileIcon, {
      name: baseName(target) || '/',
      isDirectory: true,
      icon: place.icon ?? landmarkIcon(target, props.source),
    })
}

const confirmLabel = computed(
  () =>
    props.confirmLabel ?? (props.mode === 'file' ? 'Open' : 'Select Folder'),
)
const canConfirm = computed(() => {
  if (props.confirmDisabled || props.confirmPending) {
    return false
  }
  if (selectedEntry.value) {
    return true
  }
  return props.mode === 'directory' && !failure.value
})
/** The status row: what is selected, or what can be. */
const status = computed(() => {
  const entry = selectedEntry.value
  if (entry) {
    const icon = entry.isDirectory
      ? landmarkIcon(joinPath(path.value, entry.name), props.source)
      : undefined
    return {
      lead: entry.isDirectory ? 'Selected folder:' : 'Selected file:',
      isDirectory: entry.isDirectory,
      name: entry.name,
      icon,
    }
  }
  if (props.mode === 'file') {
    return {
      lead: 'Select a file',
      isDirectory: false,
      name: null,
      icon: undefined,
    }
  }
  return {
    lead: 'Select a folder, or use',
    isDirectory: true,
    name: baseName(path.value) || '/',
    icon: landmarkIcon(path.value, props.source),
  }
})

function toFailure(err: unknown): FileBrowserFailure {
  if (err instanceof FileBrowserError) {
    return {
      kind: err.kind,
      message: err.message === err.kind ? undefined : err.message,
    }
  }
  return {
    kind: 'other',
    message: err instanceof Error ? err.message : String(err),
  }
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
    if (controller.signal.aborted) {
      return
    }
    entries.value = listed
  } catch (err) {
    if (controller.signal.aborted) {
      return
    }
    failure.value = toFailure(err)
  } finally {
    if (!controller.signal.aborted) {
      loading.value = false
    }
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
  folderCreation.value = 'idle'
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
  if (previous !== null) {
    show(previous)
  }
}

function forward() {
  const next = history.forward()
  syncHistory()
  if (next !== null) {
    show(next)
  }
}

function up() {
  if (path.value !== '/') {
    goTo(parentPath(path.value))
  }
}

/** Opens a folder; confirms a file (file mode only, the list enforces it). */
function activate(entry: FileBrowserEntry) {
  if (entry.isDirectory) {
    goTo(joinPath(path.value, entry.name))
  } else {
    emit('select', joinPath(path.value, entry.name))
  }
}

function confirm() {
  const entry = selectedEntry.value
  if (entry) {
    if (props.mode === 'file' && entry.isDirectory) {
      goTo(joinPath(path.value, entry.name))
    } else {
      emit('select', joinPath(path.value, entry.name))
    }
    return
  }
  if (props.mode === 'directory' && !failure.value) {
    emit('select', path.value)
  }
}

async function createFolder(folderName: string) {
  const create = props.source.createDirectory
  if (!create || folderCreation.value === 'saving') {
    return
  }
  folderCreation.value = 'saving'
  const target = joinPath(path.value, folderName)
  pending?.abort()
  const controller = new AbortController()
  pending = controller
  try {
    await create(target, controller.signal)
    controller.signal.throwIfAborted()
  } catch (err) {
    if (controller.signal.aborted) {
      return
    }
    error.value =
      toFailure(err).message ?? `${folderName} could not be created.`
    return
  } finally {
    if (pending === controller) {
      folderCreation.value = 'idle'
    }
  }
  await load(path.value)
  showHidden.value ||= folderName.startsWith('.')
  selected.value = folderName
  list.value?.focus()
}

watch(selected, (next) => {
  if (next !== null) {
    error.value = null
  }
})

// Another device: the history starts over at its home or the path the caller names.
watch(
  () => props.source,
  (source) => {
    const start = normalizePath(props.initialPath ?? source.home)
    history.reset(start)
    syncHistory()
    show(start)
  },
)

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
    <!-- One row at width: device, nav, path, icons. Narrow: the path takes a row of its own
         under the toolbar, whose left holds the device and nav and whose right the icons. -->
    <div class="flex shrink-0 flex-wrap items-center gap-2 py-2 pl-2 pr-3">
      <!-- The device sits in the rail's column with the rail's own inset; the path beside it starts at that device's root. -->
      <Dropdown
        v-if="hosts.length"
        :overlay-store="appOverlayStore"
        variant="field"
        fill
        trigger-label="Device"
        class="w-40 shrink-0"
      >
        <template #trigger>
          <component
            :is="currentHost ? hostIcon(currentHost) : hostIcon({ id: '' })"
            :size="ICON_PX.in28"
            class="shrink-0 text-fg-muted"
          />
          <span class="min-w-0 flex-1 truncate">{{
            currentHost?.label ?? 'Device'
          }}</span>
        </template>
        <template #content="{ triggerWidth }">
          <Menu :style="{ minWidth: `${triggerWidth}px` }">
            <MenuItem
              v-for="host in hosts"
              :key="host.id"
              :icon="hostIcon(host)"
              :indicator="
                host.id === CLOUD_HOST_ID
                  ? undefined
                  : host.online
                    ? 'success'
                    : 'muted'
              "
              :indicator-label="host.online ? 'Online' : 'Offline'"
              :note="
                host.id !== CLOUD_HOST_ID && !host.online
                  ? 'offline'
                  : undefined
              "
              :disabled="host.id !== CLOUD_HOST_ID && !host.online"
              disabled-reason="This device is offline."
              :label="host.label"
              choice
              :is-selected="host.id === hostId"
              @select="emit('update:hostId', host.id)"
            />
          </Menu>
        </template>
      </Dropdown>
      <div class="flex shrink-0 items-center">
        <Tooltip content="Back">
          <IconButton
            :icon="ArrowLeft"
            variant="ghost"
            aria-label="Back"
            :disabled="!canBack"
            @click="back"
          />
        </Tooltip>
        <Tooltip content="Forward" class="hidden @md:inline-flex">
          <IconButton
            :icon="ArrowRight"
            variant="ghost"
            aria-label="Forward"
            :disabled="!canForward"
            @click="forward"
          />
        </Tooltip>
        <Tooltip content="Up">
          <IconButton
            :icon="ArrowUp"
            variant="ghost"
            aria-label="Parent folder"
            :disabled="path === '/'"
            @click="up"
          />
        </Tooltip>
      </div>
      <span class="flex-1 @md:hidden" />
      <FileBrowserAddressBar
        :source="source"
        class="order-1 min-w-0 basis-full @md:order-none @md:flex-1 @md:basis-auto"
        :path="path"
        @navigate="goTo"
      />
      <div class="flex shrink-0 items-center gap-0.5">
        <!-- The rail's places, as a menu where the rail has no room. -->
        <Dropdown
          v-if="hasRail"
          :overlay-store="appOverlayStore"
          class="@md:hidden"
        >
          <template #trigger="{ isOpen }">
            <Tooltip content="Places">
              <IconButton
                :icon="MapPin"
                variant="ghost"
                :pressed="isOpen"
                aria-label="Places"
              />
            </Tooltip>
          </template>
          <template #content>
            <Menu>
              <MenuGroup
                v-for="(group, index) in places"
                :key="group.label ?? index"
                :label="group.label ?? 'Places'"
              >
                <MenuItem
                  v-for="place in group.places"
                  :key="place.path"
                  :icon="placeIcon(place)"
                  :label="place.label ?? (baseName(place.path) || '/')"
                  :title="place.path"
                  @select="goTo(place.path)"
                />
              </MenuGroup>
            </Menu>
          </template>
        </Dropdown>
        <Tooltip v-if="source.createDirectory" content="New folder">
          <IconButton
            :icon="FolderPlus"
            variant="ghost"
            aria-label="New folder"
            :disabled="!!failure || loading"
            @click="folderCreation = 'naming'"
          />
        </Tooltip>
        <Tooltip
          :content="showHidden ? 'Hide hidden files' : 'Show hidden files'"
        >
          <IconButton
            :icon="showHidden ? EyeOff : Eye"
            variant="ghost"
            :pressed="showHidden"
            :aria-label="showHidden ? 'Hide hidden files' : 'Show hidden files'"
            @click="showHidden = !showHidden"
          />
        </Tooltip>
      </div>
    </div>
    <div class="flex min-h-0 flex-1 border-y border-line">
      <ScrollArea
        v-if="hasRail"
        class="hidden w-44 shrink-0 border-r border-line bg-surface @md:block"
        viewport-class="flex flex-col gap-3 p-2"
      >
        <nav
          v-for="(group, index) in places"
          :key="group.label ?? index"
          class="flex flex-col gap-0.5"
          :aria-label="group.label ?? 'Places'"
        >
          <div
            v-if="group.label"
            class="px-2 pb-1 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle"
          >
            {{ group.label }}
          </div>
          <SidebarNavItem
            v-for="place in group.places"
            :key="place.path"
            :icon="placeIcon(place)"
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
        :creating="folderCreation !== 'idle'"
        :create-pending="folderCreation === 'saving'"
        @activate="activate"
        @sort="(key: FileBrowserSortKey) => (sort = nextSort(sort, key))"
        @up="up"
        @back="back"
        @forward="forward"
        @create="createFolder"
        @cancel-create="folderCreation = 'idle'"
      />
    </div>
    <div
      class="flex shrink-0 flex-col gap-2 px-3 py-2 @md:flex-row @md:items-center @md:gap-3"
    >
      <div class="flex min-w-0 flex-1 items-center gap-3">
        <div
          class="flex min-w-0 flex-1 items-center gap-2 text-chrome"
          role="status"
        >
          <InlineError v-if="error" class="min-w-0 flex-1" :message="error" />
          <template v-else>
            <span class="shrink-0 text-fg-subtle">{{ status.lead }}</span>
            <span
              v-if="status.name"
              class="flex min-w-0 items-center gap-1 text-fg"
            >
              <FileIcon
                :name="status.name"
                :is-directory="status.isDirectory"
                :icon="status.icon"
              />
              <span class="truncate" :title="status.name">{{
                status.name
              }}</span>
            </span>
          </template>
        </div>
      </div>
      <div class="flex shrink-0 items-center justify-end gap-2">
        <Button
          variant="primary"
          :disabled="!canConfirm && !confirmPending"
          :loading="confirmPending"
          @click="confirm"
          >{{ confirmLabel }}</Button
        >
        <Button @click="emit('cancel')">Cancel</Button>
      </div>
    </div>
  </div>
</template>
