<script setup lang="ts">
import { around, t } from './locales/text'
import { computed } from 'vue'
import { X } from '@lucide/vue'
import Button from '../ui/Button.vue'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'

/**
 * The version of the draft that a later save replaced, offered above the
 * composer.
 *
 * The notice shows the version's first line, Restore, which exchanges the
 * version with the draft, and a control that dismisses it. The row owns the
 * dismiss control's inset from its edges.
 */
defineProps<{
  /** The replaced version as one line. */
  preview: string
}>()

const emit = defineEmits<{
  restore: []
  dismiss: []
}>()

/**
 * The sentence around the draft's first line, which stands apart in its own
 * tone.
 */
const sentence = computed(() => around('notices.replacedDraft', 'draft'))
</script>

<template>
  <div
    class="replaced-draft-notice flex w-full items-center gap-2 rounded-lg bg-surface-card pl-3 text-chrome text-fg-muted"
    role="status"
  >
    <span class="min-w-0 flex-1 truncate">
      {{ sentence[0] }}<span class="text-fg-body">{{ t('notices.quoted', { text: preview }) }}</span>{{ sentence[1] }}
    </span>
    <Button size="sm" class="shrink-0" @click="emit('restore')">{{ t('notices.restore') }}</Button>
    <span class="replaced-draft-notice-dismiss">
      <Tooltip :content="t('notices.dismiss')">
        <IconButton
          :icon="X"
          size="sm"
          variant="ghost"
          circle
          :aria-label="t('notices.dismissDraft')"
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
  /*
   * No inline strut, so the Tooltip's wrapper cannot lift the control off
   * center.
   */
  line-height: 0;
  margin-right: calc((var(--replaced-draft-row-h) - var(--spacing-hit-sm)) / 2);
}

/* A phone's row is 44 px, around its 36 px buttons. */
@media (max-width: 767px) {
  .replaced-draft-notice {
    --replaced-draft-row-h: var(--spacing-row);
  }
}
</style>
