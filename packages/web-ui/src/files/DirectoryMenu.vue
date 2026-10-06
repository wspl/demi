<script setup lang="ts">
import { computed, h } from 'vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import FileIcon from './FileIcon.vue'
import type { FileBrowserEntry, FileBrowserSource } from './types'
import { isHiddenName, joinPath } from './paths'
import { DEFAULT_SORT, sortEntries } from './file-browser-state'
import { useShowing } from './showing'

/**
 * One directory as a menu: its entries, directories first, each directory
 * opening its own entries as a submenu, so a breadcrumb can offer the
 * files beside the one shown and everything under them. Choosing a file
 * reports it; a directory only unfolds. The listing shows while the menu
 * does, as the source keeps it.
 */
const props = defineProps<{
  source: Pick<FileBrowserSource, 'showListing'>
  /** The directory listed. */
  path: string
  /** The entry on the way to what is shown, marked as current. */
  current?: string | null
}>()

const emit = defineEmits<{
  pick: [path: string]
}>()

const shown = useShowing(() => props.source, () => props.path, (source, path) => source.showListing(path))
// A menu has no sort controls: folders first, then names.
const entries = computed(() => sortEntries(shown.entry.value?.value ?? [], DEFAULT_SORT))
const loading = computed(() => shown.entry.value?.value === undefined && shown.entry.value?.failure == null)
const failure = computed(() => shown.entry.value?.value === undefined ? shown.entry.value?.failure ?? null : null)

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
      :label="failure.kind === 'permission' ? 'No Access' : 'Unavailable'"
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
