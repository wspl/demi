<script setup lang="ts">
import { Folder, GitBranch, Monitor } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { Device, Project } from '../prototype/types'

defineProps<{ project: Project; device?: Device }>()
const emit = defineEmits<{ select: [] }>()
</script>

<template>
  <Tooltip
    :content="`${project.host} · ${project.path} · Branch: ${project.branch ?? 'unavailable'} (prototype)`"
  >
    <Button
      variant="ghost"
      class="max-w-full gap-3"
      aria-label="Change workspace"
      @click="emit('select')"
    >
      <span class="flex min-w-0 items-center gap-1.5">
        <Monitor :size="ICON_PX.in28" class="shrink-0" />
        <span class="max-w-28 truncate">{{ device?.name ?? project.host }}</span>
        <span
          v-if="device"
          class="size-1.5 shrink-0 rounded-full"
          :class="device.online ? 'bg-on-success' : 'bg-on-warning'"
          :aria-label="device.online ? 'Device online' : 'Device offline'"
        />
      </span>
      <span class="flex min-w-0 items-center gap-1.5">
        <Folder :size="ICON_PX.in28" class="shrink-0" />
        <span class="max-w-32 truncate">{{ project.name }}</span>
      </span>
      <span class="flex min-w-0 items-center gap-1.5 text-fg-subtle">
        <GitBranch :size="ICON_PX.in28" class="shrink-0" />
        <span class="max-w-32 truncate">{{ project.branch ?? 'Branch unavailable' }}</span>
      </span>
    </Button>
  </Tooltip>
</template>
