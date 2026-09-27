<script setup lang="ts">
import { X } from '@lucide/vue'
import Button from '../ui/Button.vue'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'

/**
 * The version of the draft that a later save replaced, offered above the
 * composer (`web-application.md` § Drafts): its first line, Restore, which
 * exchanges it with the draft, and a control that dismisses it. The row owns
 * the dismiss control's inset from its edges.
 */
defineProps<{
  /** The replaced version as one line. */
  preview: string
}>()

const emit = defineEmits<{
  restore: []
  dismiss: []
}>()
</script>

<template>
  <div
    class="replaced-draft-notice flex w-full items-center gap-2 rounded-lg bg-surface-raised pl-3 text-chrome text-fg-muted"
    role="status"
  >
    <span class="min-w-0 flex-1 truncate">
      A later save replaced <span class="text-fg-body">“{{ preview }}”</span>
    </span>
    <Button size="sm" class="shrink-0" @click="emit('restore')">Restore</Button>
    <span class="replaced-draft-notice-dismiss">
      <Tooltip content="Dismiss">
        <IconButton
          :icon="X"
          size="sm"
          variant="ghost"
          circle
          aria-label="Dismiss the replaced draft"
          @click="emit('dismiss')"
        />
      </Tooltip>
    </span>
  </div>
</template>

<style scoped>
.replaced-draft-notice {
  --replaced-draft-row-h: 36px;
  height: var(--replaced-draft-row-h);
}

/* The dismiss control sits as far from the row's right edge as centering
   puts it from its top and bottom: (row height - control height) / 2. */
.replaced-draft-notice-dismiss {
  display: flex;
  /* No inline strut, so the Tooltip's wrapper cannot lift the control off center. */
  line-height: 0;
  margin-right: calc((var(--replaced-draft-row-h) - var(--spacing-hit-sm)) / 2);
}
</style>
