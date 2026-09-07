<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { ChevronDown, ChevronUp, File, Folder, FolderX, Lock, WifiOff } from '@lucide/vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import TextInput from '../ui/TextInput.vue'
import { ICON_PX } from '../ui/icon-metrics'
import type { FileBrowserSort, FileBrowserSortKey } from './file-browser-state'
import { entryKind, formatBytes, formatModified } from './format'
import { isValidEntryName } from './paths'
import type { FileBrowserEntry, FileBrowserFailure, FileBrowserMode } from './types'

/**
 * The detail view: sortable columns over the rows of one directory. A click selects,
 * a double click or Enter opens a folder or confirms a file; the arrows move the
 * selection, Backspace goes up. In directory mode files are shown but cannot be picked.
 */
const props = defineProps<{
  entries: FileBrowserEntry[]
  mode: FileBrowserMode
  sort: FileBrowserSort
  loading: boolean
  failure: FileBrowserFailure | null
  /** A row for a folder about to be created; Enter confirms its name. */
  creating: boolean
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

const columns: { key: FileBrowserSortKey; label: string; class: string }[] = [
  { key: 'name', label: 'Name', class: '' },
  { key: 'modifiedAt', label: 'Date modified', class: 'hidden @md:block' },
  { key: 'size', label: 'Size', class: 'text-right' },
]

const scroll = ref<InstanceType<typeof ScrollArea>>()
const listEl = ref<HTMLElement>()
const newName = ref('')
const newNameInput = ref<InstanceType<typeof TextInput>>()

const selectedIndex = computed(() => props.entries.findIndex((entry) => entry.name === selected.value))

function pickable(entry: FileBrowserEntry) {
  return props.mode === 'file' || entry.isDirectory
}

function pick(entry: FileBrowserEntry) {
  if (!pickable(entry)) return
  selected.value = entry.name
}

function activate(entry: FileBrowserEntry) {
  if (!pickable(entry)) return
  emit('activate', entry)
}

function moveSelection(delta: number) {
  const list = props.entries
  if (!list.length) return
  let index = selectedIndex.value
  const step = delta > 0 ? 1 : -1
  for (let remaining = Math.abs(delta); remaining > 0; ) {
    index = Math.min(list.length - 1, Math.max(0, index + step))
    if (pickable(list[index]!)) remaining -= 1
    if (index === 0 || index === list.length - 1) break
  }
  if (!pickable(list[index]!)) return
  selected.value = list[index]!.name
  nextTick(() => {
    listEl.value?.querySelector<HTMLElement>('[data-selected]')?.scrollIntoView({ block: 'nearest' })
  })
}

function onKeydown(event: KeyboardEvent) {
  if (props.creating) return
  const modifier = event.altKey || event.metaKey
  if (event.key === 'ArrowDown') moveSelection(1)
  else if (event.key === 'ArrowUp') moveSelection(-1)
  else if (event.key === 'Home') moveSelection(-props.entries.length)
  else if (event.key === 'End') moveSelection(props.entries.length)
  else if (event.key === 'Enter' && selectedIndex.value >= 0) activate(props.entries[selectedIndex.value]!)
  else if (event.key === 'Backspace' || (event.key === 'ArrowUp' && modifier)) emit('up')
  else if (event.key === 'ArrowLeft' && modifier) emit('back')
  else if (event.key === 'ArrowRight' && modifier) emit('forward')
  else return
  event.preventDefault()
}

function onNewNameKeydown(event: KeyboardEvent) {
  if (event.key === 'Enter') {
    event.preventDefault()
    if (isValidEntryName(newName.value)) emit('create', newName.value.trim())
  } else if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    emit('cancelCreate')
  }
}

watch(() => props.creating, (creating) => {
  if (!creating) return
  newName.value = 'New folder'
  nextTick(() => {
    newNameInput.value?.focus()
    newNameInput.value?.select()
  })
})

// A fresh listing starts at the top.
watch(() => props.entries, (next, previous) => {
  if (next !== previous && scroll.value?.el) scroll.value.el.scrollTop = 0
})

const failureCopy = computed(() => {
  const failure = props.failure
  if (!failure) return null
  if (failure.kind === 'not-found') return { icon: FolderX, title: 'This folder does not exist.' }
  if (failure.kind === 'permission') return { icon: Lock, title: 'You do not have permission to open this folder.' }
  if (failure.kind === 'offline') return { icon: WifiOff, title: 'The device is offline.' }
  return { icon: FolderX, title: 'This folder could not be read.' }
})

defineExpose({
  focus() {
    listEl.value?.focus({ preventScroll: true })
  },
})
</script>

<template>
  <div class="@container flex min-h-0 flex-1 flex-col">
    <div
      class="grid h-7 shrink-0 grid-cols-[minmax(0,1fr)_5.5rem] items-center border-b border-line px-2 text-[12px] text-fg-subtle @md:grid-cols-[minmax(0,1fr)_8.5rem_5.5rem]"
      role="row"
    >
      <span
        v-for="column in columns"
        :key="column.key"
        role="columnheader"
        :aria-sort="sort.key === column.key ? (sort.direction === 'asc' ? 'ascending' : 'descending') : 'none'"
        class="flex h-full cursor-default items-center gap-1 px-1 transition-colors duration-200 ease-out hover:text-fg"
        :class="[column.class, column.key === 'size' ? 'justify-end' : '']"
        @click="emit('sort', column.key)"
      >
        <span class="truncate">{{ column.label }}</span>
        <component
          :is="sort.direction === 'asc' ? ChevronUp : ChevronDown"
          v-if="sort.key === column.key"
          :size="ICON_PX.in20"
          class="shrink-0"
        />
      </span>
    </div>
    <ScrollArea ref="scroll" class="min-h-0 flex-1" viewport-class="p-1">
      <div
        ref="listEl"
        class="flex min-h-full flex-col gap-px outline-none"
        role="listbox"
        :aria-label="mode === 'file' ? 'Files' : 'Folders'"
        tabindex="0"
        @keydown="onKeydown"
        @click.self="selected = null"
      >
        <div
          v-if="creating"
          class="grid h-7 shrink-0 grid-cols-[minmax(0,1fr)_5.5rem] items-center rounded-md bg-hover px-1 @md:grid-cols-[minmax(0,1fr)_8.5rem_5.5rem]"
          role="option"
          aria-selected="true"
        >
          <span class="flex min-w-0 items-center gap-2 px-1">
            <Folder :size="ICON_PX.in28" class="shrink-0 text-fg-muted" />
            <TextInput ref="newNameInput" v-model="newName" size="sm" aria-label="New folder name" spellcheck="false" @keydown="onNewNameKeydown" @blur="emit('cancelCreate')" />
          </span>
        </div>
        <div
          v-for="entry in entries"
          :key="entry.name"
          role="option"
          :aria-selected="entry.name === selected"
          :aria-disabled="!pickable(entry) || undefined"
          :data-selected="entry.name === selected || undefined"
          class="grid h-7 shrink-0 select-none grid-cols-[minmax(0,1fr)_5.5rem] items-center rounded-md px-1 text-chrome transition-colors duration-200 ease-out @md:grid-cols-[minmax(0,1fr)_8.5rem_5.5rem]"
          :class="
            entry.name === selected
              ? 'bg-active text-fg-emphasis'
              : pickable(entry)
                ? 'text-fg-body hover:bg-hover'
                : 'text-fg-faint'
          "
          @click="pick(entry)"
          @dblclick="activate(entry)"
        >
          <span class="flex min-w-0 items-center gap-2 px-1">
            <component
              :is="entry.isDirectory ? Folder : File"
              :size="ICON_PX.in28"
              class="shrink-0"
              :class="entry.name === selected ? '' : pickable(entry) ? 'text-fg-muted' : 'text-fg-faint'"
            />
            <span class="truncate" :title="entry.name">{{ entry.name }}</span>
          </span>
          <span class="hidden truncate px-1 text-[12px] text-fg-subtle @md:block">{{ formatModified(entry.modifiedAt) }}</span>
          <span class="truncate px-1 text-right text-[12px] text-fg-subtle" :title="entryKind(entry.name, entry.isDirectory)">
            {{ entry.isDirectory ? '' : entry.size != null ? formatBytes(entry.size) : '' }}
          </span>
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
          <span v-if="failure?.message" class="text-[12px] leading-4 text-fg-subtle">{{ failure.message }}</span>
        </div>
        <div
          v-else-if="!entries.length && !creating"
          class="flex flex-1 select-none items-center justify-center py-10 text-chrome text-fg-subtle"
        >
          This folder is empty.
        </div>
      </div>
    </ScrollArea>
  </div>
</template>
