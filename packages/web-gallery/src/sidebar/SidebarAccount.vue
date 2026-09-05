<script setup lang="ts">
import { computed } from 'vue'
import { ChevronsUpDown, CircleUser, LogOut, Settings } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { SidebarAccount } from './sidebar-data'

/** The signed-in user at the foot of the sidebar; the menu behind it holds settings and sign-out. */
const props = defineProps<{
  account: SidebarAccount
  collapsed?: boolean
}>()

const emit = defineEmits<{
  openSettings: []
  signOut: []
}>()

const initials = computed(() => props.account.name.trim().slice(0, 1).toUpperCase())
</script>

<template>
  <Dropdown :overlay-store="appOverlayStore" placement="top-start" :offset="8" :class="collapsed ? '' : 'w-full'">
    <template #trigger="{ isOpen }">
      <Tooltip :content="`${account.name} · ${account.plan}`" placement="right" :disabled="!collapsed" tag="div">
        <div
          role="button"
          class="flex h-9 cursor-default select-none items-center gap-2 rounded-md transition-colors duration-200 ease-out"
          :class="[
            collapsed ? 'w-9 justify-center' : 'w-full px-1.5',
            isOpen ? 'bg-hover' : 'hover:bg-hover',
          ]"
        >
          <span class="flex size-6 shrink-0 items-center justify-center rounded-full bg-tint-accent text-[11px] font-medium text-on-accent">{{ initials }}</span>
          <template v-if="!collapsed">
            <span class="flex min-w-0 flex-1 flex-col leading-4">
              <span class="truncate text-chrome text-fg">{{ account.name }}</span>
              <span class="truncate text-[11px] text-fg-subtle">{{ account.plan }}</span>
            </span>
            <ChevronsUpDown :size="ICON_PX.in28" class="shrink-0 text-fg-faint" />
          </template>
        </div>
      </Tooltip>
    </template>
    <template #content="{ close }">
      <Menu class="w-60">
        <div class="flex items-center gap-2 px-2 py-1.5">
          <CircleUser :size="ICON_PX.in28" class="shrink-0 text-fg-muted" />
          <span class="flex min-w-0 flex-col leading-4">
            <span class="truncate text-chrome text-fg">{{ account.name }}</span>
            <span class="truncate text-[11px] text-fg-subtle">{{ account.email }}</span>
          </span>
        </div>
        <MenuDivider />
        <MenuItem :icon="Settings" label="Settings" shortcut="⌘," @select="close(); emit('openSettings')" />
        <MenuDivider />
        <MenuItem :icon="LogOut" label="Sign out" @select="close(); emit('signOut')" />
      </Menu>
    </template>
  </Dropdown>
</template>
