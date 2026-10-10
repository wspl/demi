<script setup lang="ts">
import Button from '../ui/Button.vue'

/**
 * The conversation's tree runs with commands of plugins the user has since
 * turned on or off (`plugins.md` § A user's plugins): offered above the
 * composer, with Reload, which opens the conversation again with the
 * plugins on. Nothing is lost by a reload. The row owns the Reload
 * button's inset from its edges.
 */
defineProps<{
  /** The reload was asked for and has not ended yet. */
  reloading?: boolean
}>()

const emit = defineEmits<{
  reload: []
}>()
</script>

<template>
  <div
    class="plugins-changed-notice flex w-full items-center gap-2 rounded-lg bg-surface-card pl-3 text-chrome text-fg-muted"
    role="status"
  >
    <span class="min-w-0 flex-1 truncate">
      Plugins changed. Reload to update the agent’s commands.
    </span>
    <span class="plugins-changed-notice-reload">
      <Button size="sm" :disabled="reloading" @click="emit('reload')">Reload</Button>
    </span>
  </div>
</template>

<style scoped>
.plugins-changed-notice {
  --plugins-changed-row-h: 36px;
  height: var(--plugins-changed-row-h);
}

/* Reload sits as far from the row's right edge as centering puts it from
   its top and bottom: (row height - control height) / 2. */
.plugins-changed-notice-reload {
  display: flex;
  flex-shrink: 0;
  /* No inline strut, so the Tooltip's wrapper cannot lift the control off center. */
  line-height: 0;
  margin-right: calc((var(--plugins-changed-row-h) - var(--spacing-hit-sm)) / 2);
}
</style>
