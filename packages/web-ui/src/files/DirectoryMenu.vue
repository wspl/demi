<script setup lang="ts">
import { h, onBeforeUnmount, ref, watch } from 'vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import FileIcon from './FileIcon.vue'
import { FileBrowserError, type FileBrowserEntry, type FileBrowserFailure, type FileBrowserSource } from './types'
import { isHiddenName, joinPath } from './paths'
import { DEFAULT_SORT, sortEntries } from './file-browser-state'

/**
 * One directory as a menu: its entries, directories first, each directory
 * opening its own entries as a submenu, so a breadcrumb can offer the
 * files beside the one shown and everything under them. Choosing a file
 * reports it; a directory only unfolds. The listing loads when the menu
 * shows and is dropped when it goes.
 */
const props = defineProps<{
  source: Pick<FileBrowserSource, 'list'>
  /** The directory listed. */
  path: string
  /** The entry on the way to what is shown, marked as current. */
  current?: string | null
}>()

const emit = defineEmits<{
  pick: [path: string]
}>()

const entries = ref<FileBrowserEntry[]>([])
const loading = ref(true)
const failure = ref<FileBrowserFailure | null>(null)
let controller: AbortController | null = null

async function load(): Promise<void> {
  controller?.abort()
  const current = new AbortController()
  controller = current
  loading.value = true
  failure.value = null
  try {
    const list = await props.source.list(props.path, current.signal)
    if (!current.signal.aborted) {
      // A menu has no sort controls: folders first, then names.
      entries.value = sortEntries(list, DEFAULT_SORT)
    }
  } catch (error) {
    if (current.signal.aborted) {
      return
    }
    failure.value = error instanceof FileBrowserError
      ? { kind: error.kind, message: error.message }
      : { kind: 'other', message: error instanceof Error ? error.message : String(error) }
  } finally {
    if (!current.signal.aborted) {
      loading.value = false
    }
  }
}

watch(() => [props.source, props.path], load, { immediate: true })

onBeforeUnmount(() => {
  controller?.abort()
})

function iconFor(entry: FileBrowserEntry) {
  return () => h(FileIcon, { name: entry.name, isDirectory: entry.isDirectory, size: 16 })
}

function pathOf(entry: FileBrowserEntry): string {
  return joinPath(props.path, entry.name)
}

function isCurrent(entry: FileBrowserEntry): boolean {
  return props.current === pathOf(entry)
}
</script>

<template>
  <Menu>
    <div v-if="loading" class="flex h-7 items-center justify-center text-fg-faint">
      <IndeterminateSpinner :size="12" :stroke-width="1.5" />
    </div>
    <MenuItem
      v-else-if="failure"
      :label="failure.kind === 'permission' ? 'No access' : 'Unavailable'"
      :note="failure.message"
      disabled
    />
    <MenuItem v-else-if="entries.length === 0" label="Empty" disabled />
    <template v-else>
      <MenuItem
        v-for="entry in entries"
        :key="entry.name"
        :icon="iconFor(entry)"
        :label="entry.name"
        :faded="isHiddenName(entry.name)"
        :class="isCurrent(entry) ? 'text-fg-emphasis' : ''"
        @select="entry.isDirectory ? undefined : emit('pick', pathOf(entry))"
      >
        <template v-if="entry.isDirectory" #submenu>
          <DirectoryMenu
            :source="source"
            :path="pathOf(entry)"
            :current="current"
            @pick="emit('pick', $event)"
          />
        </template>
      </MenuItem>
    </template>
  </Menu>
</template>
