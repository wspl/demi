<script setup lang="ts">
import { h, inject, type FunctionalComponent } from 'vue'
import { Check } from '@lucide/vue'
import { ICON_PX } from '../ui/icon-metrics'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import { menuRootKey } from '../ui/menu-context'
import WorkspaceTile from './WorkspaceTile.vue'
import { workspaceName, type WorkspaceView } from './workspaces'

/**
 * Move To's submenu on conversations and projects: the workspaces, each with
 * its tile, a check on the one they are in. A choice closes the menu, as a
 * macOS menu's does, and choosing the one they are in moves nothing.
 */
const props = defineProps<{
  workspaces: readonly WorkspaceView[]
  /** The workspace the targets are in. */
  currentId: string | null
}>()

const emit = defineEmits<{
  choose: [id: string]
}>()

const menuRoot = inject(menuRootKey, null)

function tileOf(workspace: WorkspaceView): FunctionalComponent {
  return () => h(WorkspaceTile, { icon: workspace.icon, color: workspace.color, size: 16 })
}

function choose(id: string): void {
  menuRoot?.dismiss()
  if (id !== props.currentId) {
    emit('choose', id)
  }
}
</script>

<template>
  <Menu class="min-w-44">
    <MenuItem
      v-for="workspace in workspaces"
      :key="workspace.id"
      :icon="tileOf(workspace)"
      :label="workspaceName(workspace)"
      @select="choose(workspace.id)"
    >
      <template #suffix>
        <span class="flex size-3.5 shrink-0 items-center justify-center">
          <Check v-if="workspace.id === currentId" :size="ICON_PX.in28" class="text-fg-body" />
        </span>
      </template>
    </MenuItem>
  </Menu>
</template>
