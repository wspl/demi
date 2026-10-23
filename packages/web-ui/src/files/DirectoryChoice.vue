<script setup lang="ts">
import { Folder } from '@lucide/vue'
import MenuItem from '../ui/MenuItem.vue'
import DirectoryLabel from './DirectoryLabel.vue'
import { directoryParts } from './paths'

/**
 * One directory as a row of a menu that chooses among directories: a folder
 * icon, its name, where it is in grey at the right (`DirectoryLabel`), the
 * whole path as its tip, and a check at the end while it is the one shown.
 * The composer's directory menu and the Files tab's are made of these rows.
 */
defineProps<{
  path: string
  /** The Host's home, which `~` writes. */
  home?: string | null
  /** The name that stands for the directory's own, where it has one, such as the Cloud's Workspace. */
  name?: string
  selected: boolean
  disabled?: boolean
}>()

const emit = defineEmits<{ select: [] }>()
</script>

<template>
  <MenuItem
    :icon="Folder"
    :label="name ?? directoryParts(path, home).name"
    :hint="path"
    choice
    :is-selected="selected"
    :disabled="disabled"
    @select="emit('select')"
  >
    <DirectoryLabel :path="path" :home="home" :name="name" />
  </MenuItem>
</template>
