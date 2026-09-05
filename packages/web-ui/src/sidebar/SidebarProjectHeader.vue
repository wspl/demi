<script setup lang="ts">
import { ChevronRight, Folder, FolderOpen, SquarePen } from '@lucide/vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { SidebarProject } from './types'

/** A project row: the checkout's name and the host it is on, and the fold. */
defineProps<{
  project: SidebarProject
  collapsed: boolean
  /** The keyboard cursor is here. */
  focused: boolean
  menuOpen: boolean
}>()

const emit = defineEmits<{
  create: []
  toggle: []
  contextmenu: [event: MouseEvent]
}>()
</script>

<template>
  <Tooltip :content="`${project.host}:${project.path}`" placement="right" tag="div">
    <div
      role="button"
      :aria-expanded="!collapsed"
      class="flex h-7 cursor-default select-none items-center gap-2 rounded-md px-2 text-chrome text-fg transition-colors duration-200 ease-out"
      :class="[
        menuOpen ? 'bg-hover' : 'hover:bg-hover',
        focused ? 'ring-1 ring-inset ring-line-focus' : '',
      ]"
      @click="emit('toggle')"
      @contextmenu.prevent="emit('contextmenu', $event)"
    >
      <component
        :is="collapsed ? Folder : FolderOpen"
        :size="ICON_PX.in28"
        class="shrink-0 text-fg-muted"
      />
      <span class="flex min-w-0 flex-1 items-center gap-1.5">
        <span class="min-w-0 truncate font-medium">{{ project.name }}</span>
        <span class="min-w-0 max-w-[45%] truncate text-[11px] text-fg-subtle">
          {{ project.host }}
        </span>
      </span>
      <span class="flex shrink-0 items-center">
        <Tooltip content="New conversation">
          <IconButton
            :icon="SquarePen"
            size="sm"
            variant="ghost"
            aria-label="New conversation in project"
            @click.stop="emit('create')"
          />
        </Tooltip>
        <ChevronRight
          :size="ICON_PX.in28"
          class="shrink-0 text-fg-faint transition-transform duration-200"
          :class="collapsed ? '' : 'rotate-90'"
        />
      </span>
    </div>
  </Tooltip>
</template>
