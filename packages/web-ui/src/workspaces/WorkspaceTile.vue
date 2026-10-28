<script setup lang="ts">
/**
 * A workspace's tile: its icon, white, on its color, in a rounded square of
 * `size` px, the icon at about three fifths of it.
 */
import { computed } from 'vue'
import { workspaceFill, workspaceIconComponent, type WorkspaceColor, type WorkspaceIcon } from './workspaces'

const props = withDefaults(defineProps<{
  icon: WorkspaceIcon
  color: WorkspaceColor
  size?: number
}>(), { size: 20 })

const iconSize = computed(() => Math.round(props.size * 0.62))
const radius = computed(() => (props.size >= 24 ? 7 : props.size >= 18 ? 6 : 5))
</script>

<template>
  <span
    class="inline-flex shrink-0 items-center justify-center text-white"
    :style="{ width: `${size}px`, height: `${size}px`, borderRadius: `${radius}px`, background: workspaceFill(color) }"
    aria-hidden="true"
  >
    <component :is="workspaceIconComponent(icon)" :size="iconSize" :stroke-width="2" />
  </span>
</template>
