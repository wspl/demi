<script setup lang="ts">
import { ChevronRight, Cloud, Folder, FolderOpen, SquarePen } from '@lucide/vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import StatusDot from '@demicodes/web-ui/ui/StatusDot.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import TruncatedText from '@demicodes/web-ui/ui/TruncatedText.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { DEVICE_STATE_LABEL, DEVICE_STATE_TONE } from '@demicodes/web-ui/devices/state'
import type { SidebarProject } from './types'

/** A project row: the checkout's name, the host it is on with a device's state, and the fold. */
defineProps<{
  project: SidebarProject
  collapsed: boolean
  /** The keyboard cursor is here. */
  focused: boolean
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
    class="group/project flex h-7 cursor-default select-none items-center gap-2 rounded-md pl-2.5 pr-0.5 text-chrome text-fg transition-colors duration-200 ease-out"
    :class="[focused ? 'ring-1 ring-inset ring-line-focus' : '']"
    @click="emit('toggle')"
    @contextmenu.prevent="emit('contextmenu', $event)"
  >
    <component
      :is="collapsed ? Folder : FolderOpen"
      :size="ICON_PX.in28"
      class="shrink-0 text-fg-muted"
    />
    <!-- The text and the host's mark touch: the mark stands in its own button-sized box. -->
    <span class="flex min-w-0 flex-1 items-center">
      <!-- The project's name takes what the device's name leaves; the device's name takes its own
           width, at most half of the text's, and each is cut at its end. The hover chevron and
           the gaps around it (4px, 12px, 4px) are not text, so the device's half leaves out half of
           their 20px. -->
      <span class="flex min-w-0 flex-1 items-center gap-1">
        <span class="flex min-w-0 flex-1 items-center gap-1">
          <TruncatedText class="font-medium text-fg-emphasis" :text="project.name" />
          <ChevronRight
            :size="ICON_PX.in20"
            aria-hidden="true"
            class="shrink-0 text-fg-subtle opacity-0 transition-[opacity,rotate] duration-150 ease-out motion-reduce:transition-none group-hover/project:opacity-100"
            :class="collapsed ? 'rotate-0' : 'rotate-90'"
          />
        </span>
        <TruncatedText
          v-if="project.hostKind === 'device'"
          class="max-w-[calc(50%-10px)] flex-[0_1_auto] text-[11px] text-fg-subtle"
          :text="project.host"
        />
      </span>
      <!-- The host's mark stands where New conversation appears, so it keeps the button's inset from the row's edges. -->
      <span class="grid shrink-0 items-center">
        <span
          class="pointer-events-none col-start-1 row-start-1 flex size-hit-sm items-center justify-center text-fg-subtle transition-opacity duration-150 motion-reduce:transition-none group-hover/project:opacity-0 group-focus-within/project:opacity-0"
        >
          <Cloud
            v-if="project.hostKind === 'cloud'"
            role="img"
            :aria-label="project.host"
            :size="ICON_PX.in20"
          />
          <StatusDot
            v-else
            :tone="DEVICE_STATE_TONE[project.state]"
            :label="DEVICE_STATE_LABEL[project.state]"
          />
        </span>
        <span
          class="col-start-1 row-start-1 flex items-center justify-self-end opacity-0 pointer-events-none transition-opacity duration-150 motion-reduce:transition-none group-hover/project:opacity-100 group-hover/project:pointer-events-auto focus-within:opacity-100 focus-within:pointer-events-auto"
        >
          <Tooltip content="New conversation" class="flex items-center">
            <IconButton
              :icon="SquarePen"
              size="sm"
              variant="ghost"
              aria-label="New conversation"
              tabindex="0"
              @keydown.enter.stop.prevent="emit('create')"
              @keydown.space.stop.prevent="emit('create')"
              @click.stop="emit('create')"
            />
          </Tooltip>
        </span>
      </span>
    </span>
  </div>
</template>
