<script setup lang="ts">
import { computed } from 'vue'
import { Archive, ArrowRight, FolderInput, Pencil, Pin, PinOff, Trash2 } from '@lucide/vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import type { SidebarConversation, SidebarProject } from './types'

/** The menu for whatever is selected: one row gets open and rename, any count gets pin, move, archive, delete. */
const props = defineProps<{
  hidePin?: boolean
  hideDelete?: boolean
  targets: SidebarConversation[]
  projects: SidebarProject[]
}>()

const emit = defineEmits<{
  open: [id: string]
  rename: [id: string]
  pin: [ids: string[], pinned: boolean]
  moveTo: [ids: string[], projectId: string | null]
  archive: [ids: string[]]
  remove: [ids: string[]]
}>()

const ids = computed(() => props.targets.map((target) => target.id))
const single = computed(() => (props.targets.length === 1 ? props.targets[0] : null))
const allPinned = computed(() => props.targets.length > 0 && props.targets.every((target) => target.pinned))
const sharedProjectId = computed(() => {
  const first = props.targets[0]?.projectId ?? null
  return props.targets.every((target) => target.projectId === first) ? first : undefined
})
const many = computed(() => (props.targets.length > 1 ? ` ${props.targets.length} conversations` : ''))
</script>

<template>
  <Menu class="w-64">
    <template v-if="single">
      <MenuItem :icon="ArrowRight" label="Open" shortcut="↵" @select="emit('open', single.id)" />
      <MenuItem :icon="Pencil" label="Rename" shortcut="F2" @select="emit('rename', single.id)" />
    </template>
    <MenuItem
      v-if="!hidePin"
      :icon="allPinned ? PinOff : Pin"
      :label="`${allPinned ? 'Unpin' : 'Pin'}${many}`"
      shortcut="⌘⇧P"
      @select="emit('pin', ids, !allPinned)"
    />
    <MenuItem :icon="FolderInput">
      <span class="min-w-0 flex-1 truncate">Move to</span>
      <template #submenu>
        <Menu iconless class="min-w-[11rem]">
          <MenuItem
            label="No project"
            choice
            :is-selected="sharedProjectId === null"
            @select="emit('moveTo', ids, null)"
          />
          <MenuDivider />
          <MenuItem
            v-for="project in projects"
            :key="project.id"
            :label="project.name"
            choice
            :is-selected="sharedProjectId === project.id"
            @select="emit('moveTo', ids, project.id)"
          />
        </Menu>
      </template>
    </MenuItem>
    <MenuItem :icon="Archive" :label="`Archive${many}`" @select="emit('archive', ids)" />
    <MenuDivider v-if="!hideDelete" />
    <MenuItem v-if="!hideDelete" :icon="Trash2" :label="`Delete${many}`" shortcut="⌫" is-danger @select="emit('remove', ids)" />
  </Menu>
</template>
