<script setup lang="ts">
import { operations, type Operation } from '../ui/operations'
import { errorMessage, requestIdOf, type ShownError } from '../infra/errors'
import {
  computed,
  h,
  onBeforeUnmount,
  ref,
  watch,
  type Component,
} from 'vue'
import {
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  Eye,
  EyeOff,
  FolderPlus,
  MapPin,
} from '@lucide/vue'
import { CLOUD_HOST_ID, hostIcon } from '../hosts/icons'
import { DEVICE_STATE_LABEL, DEVICE_STATE_TONE } from '../devices/state'
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
import TruncatedText from '../ui/TruncatedText.vue'
import { ICON_PX } from '../ui/icon-metrics'
import SidebarNavItem from '../sidebar/SidebarNavItem.vue'
import FileBrowserAddressBar from './FileBrowserAddressBar.vue'
import FileBrowserList from './FileBrowserList.vue'
import DirectoryLabel from './DirectoryLabel.vue'
import FileIcon from './FileIcon.vue'
import { landmarkIcon } from './file-icons'
import {
  createFileBrowserHistory,
  DEFAULT_SORT,
  filterEntries,
  nextSort,
  sortEntries,
  type FileBrowserSort,
  type FileBrowserSortKey,
} from './file-browser-state'
import { baseName } from '@demicodes/utils'
import { directoryParts, joinPath, normalizePath, parentPath } from './paths'
import { readLinkAgain } from './link-activation'
import { linkSentence } from './format'
import { t as shared } from '../locales/text'
import { t } from './locales/text'
import { failureShown } from './file-cache'
import { useShowing } from './showing'
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
 * The file browser reads through `source` and owns everything else: where it is, what is
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
    /** What the confirm button does (`operations.ts`): it shows it while it works. */
    confirmOperation?: Operation
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
// The directory's listing as the source keeps it: a directory seen a moment
// ago shows at once, and one read again replaces its rows in place.
const listing = useShowing(() => props.source, () => path.value, (source, shown) => source.showListing(shown))
const entries = computed(() => listing.entry.value?.value ?? [])
const loading = computed(() => listing.entry.value?.answer == null && listing.entry.value?.failure == null)
const failure = computed(() => failureShown(listing.entry.value))
const sort = ref<FileBrowserSort>(DEFAULT_SORT)
const selected = ref<string | null>(null)
const error = ref<ShownError | null>(null)
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
/** A place's name: the label its caller gave it, or its directory's own. */
function placeName(place: FileBrowserPlace): string {
  return place.label ?? directoryParts(place.path, props.source.home).name
}
/** The names that more than one place goes by, which the rail tells apart by location. */
const sharedNames = computed(() => {
  const seen = new Map<string, Set<string>>()
  for (const group of props.places) {
    for (const place of group.places) {
      const name = placeName(place)
      seen.set(name, (seen.get(name) ?? new Set()).add(normalizePath(place.path)))
    }
  }
  return new Set([...seen].filter(([, paths]) => paths.size > 1).map(([name]) => name))
})
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
    props.confirmLabel ?? (props.mode === 'file' ? t('browser.open') : t('browser.selectFolder')),
)
const canConfirm = computed(() => {
  if (props.confirmDisabled || (props.confirmOperation !== undefined && operations.runs(props.confirmOperation))) {
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
      lead: entry.isDirectory ? t('browser.selectedFolder') : t('browser.selectedFile'),
      isDirectory: entry.isDirectory,
      name: entry.name,
      icon,
    }
  }
  if (props.mode === 'file') {
    return {
      lead: t('browser.selectFile'),
      isDirectory: false,
      name: null,
      icon: undefined,
    }
  }
  return {
    lead: t('browser.selectFolderOrUse'),
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
    message: errorMessage(err),
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
}

/** Shows the folder at `target`; the one shown now is listed again, as its path entered again asks. */
function goTo(target: string) {
  const next = normalizePath(target)
  if (next === path.value) {
    void listing.retry()
  } else {
    history.push(next)
    syncHistory()
  }
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

// Ends a link's reading again when the browser goes away, or to another source or folder.
let following = new AbortController()
watch([() => props.source, path], () => {
  following.abort()
  following = new AbortController()
})
onBeforeUnmount(() => following.abort())

/**
 * The entry as it is now: a link's folder is listed again, which reads its
 * target, since its row says what the target was when it was listed
 * (`readLinkAgain`); undefined when the browser moved on meanwhile.
 */
async function current(entry: FileBrowserEntry): Promise<FileBrowserEntry | undefined> {
  if (!entry.link)
    return entry
  const { signal } = following
  try {
    const now = await readLinkAgain(props.source, joinPath(path.value, entry.name), signal)
    return signal.aborted ? undefined : now
  } catch (failure) {
    // Its folder could not be listed again: said where the browser says what
    // went wrong, and nothing is opened or chosen.
    if (!signal.aborted)
      error.value = { message: toFailure(failure).message ?? t('browser.linkReadFailed'), requestId: requestIdOf(failure) }
    return undefined
  }
}

/**
 * Answers a file chosen in file mode: a link whose target is missing or
 * cannot be read, as it is now, gives nothing, and its sentence shows where
 * the browser shows its errors (`file-previews.md` § Symbolic links).
 */
function choose(entry: FileBrowserEntry): void {
  if (entry.link?.broken) {
    error.value = { message: linkSentence(entry.link), requestId: null }
    return
  }
  emit('select', joinPath(path.value, entry.name))
}

/** Opens a folder; confirms a file in file mode; a link as what it leads to now. */
async function activate(entry: FileBrowserEntry) {
  const now = await current(entry)
  if (!now)
    return
  if (now.isDirectory) {
    goTo(joinPath(path.value, now.name))
  } else if (props.mode === 'file') {
    choose(now)
  }
}

async function confirm() {
  const entry = selectedEntry.value
  if (entry) {
    const now = await current(entry)
    if (!now)
      return
    if (props.mode === 'file' && now.isDirectory) {
      goTo(joinPath(path.value, now.name))
    } else if (props.mode === 'file') {
      choose(now)
    } else if (now.isDirectory) {
      emit('select', joinPath(path.value, now.name))
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
    error.value = {
      message: toFailure(err).message ?? t('browser.folderNotCreated', { name: folderName }),
      requestId: requestIdOf(err),
    }
    return
  } finally {
    if (pending === controller) {
      folderCreation.value = 'idle'
    }
  }
  // The source's write has the listing read again.
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

onBeforeUnmount(() => pending?.abort())
</script>

<template>
  <div class="@container flex min-h-0 flex-1 select-none flex-col">
    <!-- One row at width: device, nav, path, icons. Narrow: the path takes a row of its own
         under the toolbar, whose left holds the device and nav and whose right the icons. -->
    <div class="flex shrink-0 flex-wrap items-center gap-2 py-2 pl-2 pr-3">
      <!-- The device sits in the rail's column with the rail's own inset; the path beside it
           starts at that device's root. The column sets the width; the dropdown fills it. -->
      <div v-if="hosts.length" class="w-40 shrink-0">
        <Dropdown
          :overlay-store="appOverlayStore"
          variant="field"
          width="fill"
          :trigger-label="t('browser.device')"
        >
          <template #trigger>
            <component
              :is="currentHost ? hostIcon(currentHost) : hostIcon({ kind: 'device' })"
              :size="ICON_PX.in28"
              class="shrink-0 text-fg-muted"
            />
            <span class="min-w-0 flex-1 truncate">{{
              currentHost?.label ?? t('browser.device')
            }}</span>
          </template>
          <template #content="{ triggerWidth }">
            <Menu :style="{ minWidth: `${triggerWidth}px` }">
              <MenuItem
                v-for="host in hosts"
                :key="host.id"
                :icon="hostIcon(host)"
                :indicator="
                  host.id === CLOUD_HOST_ID || host.canWake
                    ? undefined
                    : DEVICE_STATE_TONE[host.state]
                "
                :indicator-label="DEVICE_STATE_LABEL[host.state]"
                :value="
                  host.id !== CLOUD_HOST_ID && !host.canWake && host.state !== 'online'
                    ? DEVICE_STATE_LABEL[host.state]
                    : undefined
                "
                :disabled="host.id !== CLOUD_HOST_ID && host.state !== 'online' && !host.canWake"
                :disabled-reason="t('browser.deviceOffline')"
                :label="host.label"
                choice
                :is-selected="host.id === hostId"
                @select="emit('update:hostId', host.id)"
              />
            </Menu>
          </template>
        </Dropdown>
      </div>
      <div class="flex shrink-0 items-center">
        <Tooltip :content="t('actions.back')">
          <IconButton
            :icon="ArrowLeft"
            variant="ghost"
            :aria-label="t('actions.back')"
            :disabled="!canBack"
            @click="back"
          />
        </Tooltip>
        <Tooltip :content="t('actions.forward')" class="hidden @md:inline-flex">
          <IconButton
            :icon="ArrowRight"
            variant="ghost"
            :aria-label="t('actions.forward')"
            :disabled="!canForward"
            @click="forward"
          />
        </Tooltip>
        <Tooltip :content="t('browser.up')">
          <IconButton
            :icon="ArrowUp"
            variant="ghost"
            :aria-label="t('browser.up')"
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
            <Tooltip :content="t('browser.places')">
              <IconButton
                :icon="MapPin"
                variant="ghost"
                :pressed="isOpen"
                :aria-label="t('browser.places')"
              />
            </Tooltip>
          </template>
          <template #content>
            <Menu>
              <MenuGroup
                v-for="(group, index) in places"
                :key="group.label ?? index"
                :label="group.label ?? t('browser.places')"
              >
                <MenuItem
                  v-for="place in group.places"
                  :key="place.path"
                  :icon="placeIcon(place)"
                  :label="placeName(place)"
                  :hint="place.path"
                  @select="goTo(place.path)"
                >
                  <DirectoryLabel :path="place.path" :home="source.home" :name="placeName(place)" />
                </MenuItem>
              </MenuGroup>
            </Menu>
          </template>
        </Dropdown>
        <Tooltip v-if="source.createDirectory" :content="t('browser.newFolder')">
          <IconButton
            :icon="FolderPlus"
            variant="ghost"
            :aria-label="t('browser.newFolder')"
            :disabled="!!failure || loading"
            @click="folderCreation = 'naming'"
          />
        </Tooltip>
        <Tooltip
          :content="showHidden ? t('browser.hideHidden') : t('browser.showHidden')"
        >
          <IconButton
            :icon="showHidden ? EyeOff : Eye"
            variant="ghost"
            :pressed="showHidden"
            :aria-label="showHidden ? t('browser.hideHidden') : t('browser.showHidden')"
            @click="showHidden = !showHidden"
          />
        </Tooltip>
      </div>
    </div>
    <div class="flex min-h-0 flex-1 border-y border-line">
      <ScrollArea
        v-if="hasRail"
        class="hidden w-44 shrink-0 border-r border-line bg-surface @md:flex"
        viewport-class="flex flex-col gap-3 p-2"
      >
        <nav
          v-for="(group, index) in places"
          :key="group.label ?? index"
          class="flex flex-col gap-0.5"
          :aria-label="group.label ?? t('browser.places')"
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
            :label="placeName(place)"
            :pressed="path === normalizePath(place.path)"
            :hint="place.path"
            @click="goTo(place.path)"
          >
            <DirectoryLabel
              :path="place.path"
              :home="source.home"
              :name="placeName(place)"
              :hide-location="!sharedNames.has(placeName(place))"
            />
          </SidebarNavItem>
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
          <InlineError v-if="error" class="min-w-0 flex-1" :message="error.message" :request-id="error.requestId" />
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
              <TruncatedText :text="status.name" />
            </span>
          </template>
        </div>
      </div>
      <!-- Cancel before the default button, as a dialog's footer has them (Dialog). -->
      <div class="flex shrink-0 items-center justify-end gap-2">
        <Button @click="emit('cancel')">{{ shared('common.cancel') }}</Button>
        <Button
          variant="primary"
          :disabled="!canConfirm && !(confirmOperation && operations.runs(confirmOperation))"
          :operation="confirmOperation"
          @click="confirm"
          >{{ confirmLabel }}</Button
        >
      </div>
    </div>
  </div>
</template>
