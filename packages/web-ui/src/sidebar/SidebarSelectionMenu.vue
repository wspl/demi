<script setup lang="ts">
import { computed } from 'vue'
import { Archive, ArrowRight, Copy, FolderInput, Pin, PinOff, TextCursorInput } from '@lucide/vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import type { SidebarConversation, SidebarProject } from './types'

/** One row gets open, rename and copy ID; any count gets pin, move and archive. Conversations are never deleted. */
const props = defineProps<{
  targets: SidebarConversation[]
  projects: SidebarProject[]
}>()

const emit = defineEmits<{
  open: [id: string]
  rename: [id: string]
  copyId: [id: string]
  pin: [ids: string[], pinned: boolean]
  moveTo: [ids: string[], projectId: string | null]
  archive: [ids: string[]]
}>()

const ids = computed(() => props.targets.map((target) => target.id))
const single = computed(() => (props.targets.length === 1
  ? props.targets[0]
  : null))
const allPinned = computed(
  () => props.targets.length > 0 && props.targets.every((target) => target.pinned)
)
const sharedProjectId = computed(() => {
  const first = props.targets[0]?.projectId ?? null
  return props.targets.every((target) => target.projectId === first)
    ? first
    : undefined
})
const many = computed(
  () => (props.targets.length > 1
    ? ` ${props.targets.length} conversations`
    : '')
)
</script>

<template>
  <Menu>
    <template v-if="single">
      <MenuItem
        :icon="ArrowRight"
        label="Open"
        shortcut="↵"
        @select="emit('open', single.id)"
      />
      <MenuItem
        :icon="TextCursorInput"
        label="Rename"
        shortcut="F2"
        @select="emit('rename', single.id)"
      />
      <MenuItem
        :icon="Copy"
        label="Copy conversation ID"
        @select="emit('copyId', single.id)"
      />
    </template>
    <MenuItem
      :icon="allPinned ? PinOff : Pin"
      :label="`${allPinned ? 'Unpin' : 'Pin'}${many}`"
      shortcut="⌘⇧P"
      @select="emit('pin', ids, !allPinned)"
    />
    <MenuItem :icon="FolderInput">
      <span class="min-w-0 flex-1 truncate">Move to</span>
      <template #submenu>
        <Menu iconless>
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
    <MenuItem
      :icon="Archive"
      :label="`Archive${many}`"
      @select="emit('archive', ids)"
    />
  </Menu>
</template>
