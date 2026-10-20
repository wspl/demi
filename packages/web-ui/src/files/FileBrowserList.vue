<script setup lang="ts">
import type { HeadlineText, TitleText } from '../ui/ui-text'
import { computed, nextTick, ref, useId, watch, type Component } from 'vue'
import {
  ChevronDown,
  ChevronUp,
  ChevronsUpDown,
  FolderX,
  Lock,
  WifiOff,
} from '@lucide/vue'
import { useTypeSelect } from '../composables/useTypeSelect'
import { focusedByKeyboard } from '../ui/active-row'
import { focusByPage } from '../ui/focus-by-page'
import HighlightText from '../ui/HighlightText.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import TextInput from '../ui/TextInput.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import TypeSelectHint from '../ui/TypeSelectHint.vue'
import FileIcon from './FileIcon.vue'
import LinkMark from './LinkMark.vue'
import { ICON_PX } from '../ui/icon-metrics'
import type { FileBrowserSort, FileBrowserSortKey } from './file-browser-state'
import { formatModified } from '../ui/dates'
import { entryKind, formatBytes } from './format'
import { t } from './locales/text'
import { isHiddenName, isValidEntryName } from './paths'
import type {
  FileBrowserEntry,
  FileBrowserFailure,
  FileBrowserMode,
} from './types'

/**
 * The detail view: sortable columns over the rows of one directory.
 *
 * - A click selects, a double click or Enter opens a folder or confirms a file.
 * - The arrows move the selection, Backspace goes up.
 * - Typing a name selects the first row that starts with it (`useTypeSelect`),
 *   Backspace then taking back a letter.
 * - In directory mode files are shown but cannot be picked, nor reached by typing.
 */
/** An entry with the glyph its place on the tree earns, when it earns one. */
export interface FileBrowserRow extends FileBrowserEntry {
  icon?: string
}

const props = defineProps<{
  entries: FileBrowserRow[]
  mode: FileBrowserMode
  sort: FileBrowserSort
  loading: boolean
  failure: FileBrowserFailure | null
  /** A row for a folder about to be created; Enter confirms its name. */
  creating: boolean
  createPending?: boolean
}>()

const selected = defineModel<string | null>('selected', { default: null })

const emit = defineEmits<{
  activate: [entry: FileBrowserEntry]
  sort: [key: FileBrowserSortKey]
  up: []
  back: []
  forward: []
  create: [name: string]
  cancelCreate: []
}>()

const columns = computed<{
  key: FileBrowserSortKey
  label: TitleText
  class: string
}[]>(() => [
  { key: 'name', label: t('list.name'), class: '' },
  { key: 'modifiedAt', label: t('list.dateModified'), class: 'hidden @md:flex' },
  { key: 'size', label: t('list.size'), class: 'text-right' },
])

const scroll = ref<InstanceType<typeof ScrollArea>>()
const listEl = ref<HTMLElement>()
const newName = ref('')
const newNameInput = ref<InstanceType<typeof TextInput>>()

const selectedIndex = computed(() =>
  props.entries.findIndex((entry) => entry.name === selected.value),
)

// The selected row is the list's active row: it wears the ring while the keyboard has the list.
const listId = useId()
const keyboard = ref(false)

/** Whether the row is what the browser picks, as its listing says: any row for a file, a folder for a folder. */
function pickable(entry: FileBrowserEntry) {
  return props.mode === 'file' || entry.isDirectory
}

/**
 * Whether a double click opens the row: one that can be picked, or a link, which
 * may lead elsewhere than its row said, and is read again by the browser to act on
 * what it is now.
 *
 * A link shows, and is selected, as what it was listed as. In folder mode, one
 * listed as a file is dimmed, and a double click finds whether it leads to a folder
 * now.
 */
function reachable(entry: FileBrowserEntry) {
  return pickable(entry) || entry.link !== undefined
}

function pick(entry: FileBrowserEntry) {
  if (!pickable(entry)) {
    return
  }
  selected.value = entry.name
}

function activate(entry: FileBrowserEntry) {
  if (!reachable(entry)) {
    return
  }
  emit('activate', entry)
}

/** Scrolls the selected row into view once it shows as selected. */
function revealSelected() {
  nextTick(() => {
    listEl.value
      ?.querySelector<HTMLElement>('[data-selected]')
      ?.scrollIntoView({ block: 'nearest' })
  })
}

// Typing reaches the rows a click could select.
const pickableEntries = computed(() => props.entries.filter(pickable))
const typeSelect = useTypeSelect({
  names: () => pickableEntries.value.map((entry) => entry.name),
  select: (index) => {
    selected.value = pickableEntries.value[index]!.name
    revealSelected()
  },
})

/** The letters to mark on a row's name: the typed prefix, on the selected row only. */
function highlightOf(entry: FileBrowserEntry): readonly number[] | undefined {
  return entry.name === selected.value ? (typeSelect.prefix(entry.name) ?? undefined) : undefined
}

function moveSelection(delta: number) {
  const list = props.entries
  if (!list.length) {
    return
  }
  let index = selectedIndex.value
  const step = delta > 0 ? 1 : -1
  for (let remaining = Math.abs(delta); remaining > 0;) {
    index = Math.min(list.length - 1, Math.max(0, index + step))
    if (pickable(list[index]!)) remaining -= 1
    if (index === 0 || index === list.length - 1) break
  }
  if (!pickable(list[index]!)) {
    return
  }
  selected.value = list[index]!.name
  revealSelected()
}

/** The list took focus from the keyboard: with nothing selected, the first row you can pick is. */
function onFocus(event: FocusEvent) {
  if (!focusedByKeyboard(event)) {
    return
  }
  keyboard.value = true
  if (selectedIndex.value < 0) {
    const first = props.entries.find(pickable)
    if (first) {
      selected.value = first.name
    }
  }
}

function onKeydown(event: KeyboardEvent) {
  if (props.creating) {
    return
  }
  keyboard.value = true
  if (typeSelect.keydown(event)) {
    event.preventDefault()
    return
  }
  const modifier = event.altKey || event.metaKey
  if (event.key === 'ArrowDown') moveSelection(1)
  else if (event.key === 'ArrowUp') moveSelection(-1)
  else if (event.key === 'Home') moveSelection(-props.entries.length)
  else if (event.key === 'End') moveSelection(props.entries.length)
  else if (event.key === 'Enter' && selectedIndex.value >= 0)
    activate(props.entries[selectedIndex.value]!)
  else if (event.key === 'Backspace' || (event.key === 'ArrowUp' && modifier))
    emit('up')
  else if (event.key === 'ArrowLeft' && modifier) emit('back')
  else if (event.key === 'ArrowRight' && modifier) emit('forward')
  else return
  event.preventDefault()
}

function onNewNameKeydown(event: KeyboardEvent) {
  if (props.createPending) {
    event.preventDefault()
    return
  }
  if (event.key === 'Enter') {
    event.preventDefault()
    if (isValidEntryName(newName.value)) emit('create', newName.value.trim())
  } else if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    emit('cancelCreate')
  }
}

watch(
  () => props.creating,
  (creating) => {
    if (!creating) {
      return
    }
    newName.value = t('list.newFolderDefault')
    nextTick(() => {
      newNameInput.value?.focus()
      newNameInput.value?.select()
    })
  },
)

// A fresh listing starts at the top.
watch(
  () => props.entries,
  (next, previous) => {
    if (next !== previous && scroll.value?.el) scroll.value.el.scrollTop = 0
  },
)

const failureCopy = computed<{ icon: Component; title: HeadlineText } | null>(() => {
  const failure = props.failure
  if (!failure) {
    return null
  }
  if (failure.kind === 'not-found')
    return {
      icon: FolderX,
      title: t('list.notFound'),
    }
  if (failure.kind === 'permission')
    return {
      icon: Lock,
      title: t('list.noPermission'),
    }
  if (failure.kind === 'offline')
    return {
      icon: WifiOff,
      title: t('list.offline'),
    }
  return { icon: FolderX, title: t('list.unreadable') }
})

defineExpose({
  focus() {
    if (listEl.value)
      focusByPage(listEl.value, { preventScroll: true })
  },
})
</script>

<template>
  <div class="@container flex min-h-0 flex-1 flex-col">
    <div
      class="grid h-7 shrink-0 grid-cols-[minmax(0,1fr)_5.5rem] items-center border-b border-line px-2 text-detail phone:h-11 text-fg-subtle @md:grid-cols-[minmax(0,1fr)_10rem_5.5rem]"
      role="row"
    >
      <span
        v-for="column in columns"
        :key="column.key"
        role="columnheader"
        :aria-sort="
          sort.key === column.key
            ? sort.direction === 'asc'
              ? 'ascending'
              : 'descending'
            : 'none'
        "
        class="flex h-full cursor-default items-center gap-1 px-1 transition-colors duration-200 ease-out hover:text-fg"
        :class="[column.class, column.key === 'size' ? 'justify-end' : '']"
        @click="emit('sort', column.key)"
      >
        <span class="truncate">{{ column.label }}</span>
        <!-- Every column sorts: the unsorted ones say so with a faint pair of chevrons. -->
        <component
          :is="
            sort.key === column.key
              ? sort.direction === 'asc'
                ? ChevronUp
                : ChevronDown
              : ChevronsUpDown
          "
          :size="ICON_PX.in20"
          class="shrink-0"
          :class="sort.key === column.key ? '' : 'text-fg-faint'"
        />
      </span>
    </div>
    <ScrollArea ref="scroll" class="min-h-0 flex-1" viewport-class="p-1">
      <div
        ref="listEl"
        class="flex min-h-full flex-col gap-px outline-none"
        role="listbox"
        :aria-label="mode === 'file' ? t('list.files') : t('list.folders')"
        tabindex="0"
        :aria-activedescendant="selectedIndex >= 0 ? `${listId}-${selectedIndex}` : undefined"
        @focus="onFocus"
        @blur="keyboard = false"
        @pointerdown="keyboard = false"
        @keydown="onKeydown"
        @click.self="selected = null"
      >
        <div
          v-if="creating"
          class="grid h-7 shrink-0 grid-cols-[minmax(0,1fr)_5.5rem] items-center rounded-md bg-hover px-1 phone:h-row @md:grid-cols-[minmax(0,1fr)_10rem_5.5rem]"
          role="option"
          aria-selected="true"
        >
          <span class="on-fill flex min-w-0 items-center gap-2 px-1">
            <IndeterminateSpinner v-if="createPending" :size="14" />
            <FileIcon v-else :name="newName" :is-directory="true" />
            <TextInput
              ref="newNameInput"
              v-model="newName"
              :disabled="createPending"
              size="sm"
              :aria-label="t('list.newFolderName')"
              literal
              @keydown="onNewNameKeydown"
              @blur="!createPending && emit('cancelCreate')"
            />
          </span>
        </div>
        <div
          v-for="(entry, index) in entries"
          :id="`${listId}-${index}`"
          :key="entry.name"
          role="option"
          :aria-selected="entry.name === selected"
          :aria-disabled="!reachable(entry) || undefined"
          :data-selected="entry.name === selected || undefined"
          class="grid h-7 shrink-0 select-none grid-cols-[minmax(0,1fr)_5.5rem] items-center rounded-md px-1 text-chrome phone:h-row phone:text-[16px] transition-colors duration-200 ease-out @md:grid-cols-[minmax(0,1fr)_10rem_5.5rem]"
          :class="
            entry.name === selected
              ? ['bg-active text-fg-emphasis', keyboard ? 'focus-ring-shown' : '']
              : pickable(entry)
                ? 'text-fg-body hover:bg-hover'
                : reachable(entry) ? 'text-fg-faint hover:bg-hover' : 'text-fg-faint'
          "
          @click="pick(entry)"
          @dblclick="activate(entry)"
        >
          <span class="flex min-w-0 items-center gap-2 px-1">
            <FileIcon
              :name="entry.name"
              :is-directory="entry.isDirectory"
              :icon="entry.icon"
              :class="!pickable(entry) ? 'opacity-40' : isHiddenName(entry.name) ? 'faded' : ''"
            />
            <TruncatedText :class="isHiddenName(entry.name) ? 'faded' : ''" :text="entry.name">
              <HighlightText :text="entry.name" :indexes="highlightOf(entry)" />
            </TruncatedText>
            <LinkMark v-if="entry.link" class="ml-auto" :class="!pickable(entry) ? 'opacity-40' : ''" :link="entry.link" />
          </span>
          <span
            class="hidden truncate px-1 text-detail text-fg-subtle @md:block"
            >{{ formatModified(entry.modifiedAt) }}</span
          >
          <Tooltip
            class="truncate px-1 text-right text-detail text-fg-subtle"
            :content="entryKind(entry.name, entry.isDirectory)"
          >
            {{
              entry.isDirectory
                ? ''
                : entry.size != null
                  ? formatBytes(entry.size)
                  : ''
            }}
          </Tooltip>
        </div>
        <div
          v-if="loading && !entries.length"
          class="flex flex-1 select-none items-center justify-center py-10 text-fg-subtle"
        >
          <IndeterminateSpinner :size="16" />
        </div>
        <div
          v-else-if="failureCopy"
          class="flex flex-1 select-none flex-col items-center justify-center gap-2 px-6 py-10 text-center"
          role="alert"
        >
          <component :is="failureCopy.icon" :size="20" class="text-fg-faint" />
          <span class="text-chrome text-fg-muted">{{ failureCopy.title }}</span>
          <span
            v-if="failure?.message"
            class="text-detail leading-4 text-fg-subtle"
            >{{ failure.message }}</span
          >
        </div>
        <div
          v-else-if="!entries.length && !creating"
          class="flex flex-1 select-none items-center justify-center py-10 text-chrome text-fg-subtle"
        >
          {{ t('list.empty') }}
        </div>
      </div>
      <TypeSelectHint :query="typeSelect.query.value" :matched="typeSelect.matched.value" />
    </ScrollArea>
  </div>
</template>
