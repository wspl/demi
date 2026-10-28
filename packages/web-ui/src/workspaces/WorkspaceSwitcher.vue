<script setup lang="ts">
/**
 * The sidebar's workspace switcher: a row under the app's name with the
 * current workspace's tile and name, and a dot while another workspace has a
 * conversation that waits for the user. Its menu lists the workspaces, each
 * with the keys that switch to it, then New Workspace and Workspace Settings.
 * The row shows with one workspace too, as the place another is made.
 */
import { computed, h, type FunctionalComponent } from 'vue'
import { Check, ChevronsUpDown, Plus, Settings } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import StatusDot from '../ui/StatusDot.vue'
import { pressOnKey } from '../ui/button-events'
import { ICON_PX } from '../ui/icon-metrics'
import WorkspaceTile from './WorkspaceTile.vue'
import { t } from './locales/text'
import { workspaceName, type WorkspaceView } from './workspaces'

const props = defineProps<{
  workspaces: readonly WorkspaceView[]
  /** The workspace the tab shows. */
  currentId: string | null
  /** The workspaces with a conversation that waits for the user. */
  attention: readonly string[]
  /** The keys that switch to the workspace at each place of the list, as the keyboard settings name them. */
  shortcuts?: readonly (string | undefined)[]
}>()

const emit = defineEmits<{
  switch: [id: string]
  create: []
  openSettings: []
}>()

const current = computed(() => props.workspaces.find((workspace) => workspace.id === props.currentId) ?? props.workspaces[0] ?? null)
/** Another workspace than the shown one waits for the user. */
const elsewhere = computed(() => props.attention.some((id) => id !== current.value?.id))

/** A workspace's tile where a menu row draws its icon. */
function tileOf(workspace: WorkspaceView): FunctionalComponent {
  return () => h(WorkspaceTile, { icon: workspace.icon, color: workspace.color, size: 16 })
}
</script>

<template>
  <Dropdown :overlay-store="appOverlayStore" side="bottom" :offset="4" width="fill">
    <template #trigger="{ isOpen }">
      <div
        role="button"
        tabindex="0"
        :aria-label="t('switcher')"
        class="flex h-9 w-full cursor-default select-none items-center gap-2 rounded-lg border border-line-subtle px-2 transition-colors duration-200 ease-out phone:h-11"
        :class="isOpen ? 'bg-active' : 'bg-hover hover:bg-active'"
        @keydown="pressOnKey"
        @keyup="pressOnKey"
      >
        <WorkspaceTile v-if="current" :icon="current.icon" :color="current.color" :size="20" />
        <span class="min-w-0 flex-1 truncate text-chrome font-medium text-fg-emphasis phone:text-[16px]">
          {{ current ? workspaceName(current) : '' }}
        </span>
        <!-- The dot keeps its place while no other workspace waits, so the name never moves. -->
        <StatusDot :tone="elsewhere ? 'warning' : null" :label="elsewhere ? t('needsYou') : undefined" />
        <ChevronsUpDown :size="ICON_PX.in28" class="shrink-0 text-fg-muted" />
      </div>
    </template>
    <template #content="{ close }">
      <Menu>
        <MenuItem
          v-for="(workspace, index) in workspaces"
          :key="workspace.id"
          :icon="tileOf(workspace)"
          :label="workspaceName(workspace)"
          :indicator="workspace.id !== current?.id && attention.includes(workspace.id) ? 'warning' : undefined"
          :indicator-label="t('needsYou')"
          @select="close(); emit('switch', workspace.id)"
        >
          <template #suffix>
            <span class="flex items-center gap-2">
              <span v-if="shortcuts?.[index]" class="text-[11px] text-fg-subtle phone:hidden">{{ shortcuts[index] }}</span>
              <span class="flex size-3.5 shrink-0 items-center justify-center">
                <Check v-if="workspace.id === current?.id" :size="ICON_PX.in28" class="text-fg-body" />
              </span>
            </span>
          </template>
        </MenuItem>
        <MenuDivider />
        <MenuItem :icon="Plus" :label="t('new')" @select="close(); emit('create')" />
        <MenuItem :icon="Settings" :label="t('settings')" @select="close(); emit('openSettings')" />
      </Menu>
    </template>
  </Dropdown>
</template>
