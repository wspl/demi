<script setup lang="ts">
import { ChevronRight, Cloud, Folder, FolderOpen, SquarePen } from '@lucide/vue'
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
  <div
    role="button"
    :aria-expanded="!collapsed"
    class="group/project flex h-7 cursor-default select-none items-center gap-2 rounded-md pl-2 pr-1 text-chrome text-fg transition-colors duration-200 ease-out"
    :class="[focused ? 'ring-1 ring-inset ring-line-focus' : '']"
    @click="emit('toggle')"
    @contextmenu.prevent="emit('contextmenu', $event)"
  >
    <component
      :is="collapsed ? Folder : FolderOpen"
      :size="ICON_PX.in28"
      class="shrink-0 text-fg-muted"
    />
    <span class="flex min-w-0 flex-1 items-center gap-1">
      <span class="min-w-0 truncate font-medium">{{ project.name }}</span>
      <ChevronRight
        :size="ICON_PX.in20"
        aria-hidden="true"
        class="shrink-0 text-fg-subtle opacity-0 transition-[opacity,transform] duration-150 motion-reduce:transition-none group-hover/project:opacity-100"
        :class="collapsed ? '' : 'rotate-90'"
      />
    </span>
    <span class="grid min-w-6 max-w-[45%] items-center">
      <span
        class="pointer-events-none col-start-1 row-start-1 flex min-w-0 items-center justify-end gap-1 text-[11px] leading-none text-fg-subtle transition-opacity duration-150 motion-reduce:transition-none group-hover/project:opacity-0 group-focus-within/project:opacity-0"
        :aria-label="project.host"
      >
        <Cloud v-if="project.hostKind === 'cloud'" :size="ICON_PX.in20" class="shrink-0" />
        <span v-if="project.hostKind !== 'cloud'" class="truncate">{{ project.host }}</span>
      </span>
      <span
        class="col-start-1 row-start-1 flex items-center justify-self-end opacity-0 pointer-events-none transition-opacity duration-150 motion-reduce:transition-none group-hover/project:opacity-100 group-hover/project:pointer-events-auto focus-within:opacity-100 focus-within:pointer-events-auto"
      >
        <Tooltip content="New conversation" class="flex items-center">
          <IconButton
            :icon="SquarePen"
            size="sm"
            variant="ghost"
            aria-label="New conversation in project"
            tabindex="0"
            @keydown.enter.stop.prevent="emit('create')"
            @keydown.space.stop.prevent="emit('create')"
            @click.stop="emit('create')"
          />
        </Tooltip>
      </span>
    </span>
  </div>
</template>
