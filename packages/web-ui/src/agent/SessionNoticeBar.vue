<script setup lang="ts">
import { CircleX } from '@lucide/vue'
import Button from '../ui/Button.vue'

/**
 * A condition that applies to the whole session or page while it lasts:
 * archived, no model can send, the connection dropped, a request was refused.
 *
 * `neutral` describes a state the reader chose or can leave (archived, no
 * models). `danger` describes a failure that persists until the session
 * recovers. Either tone offers at most one action. A bar never repeats what a
 * transcript record or a status pane already says.
 */
withDefaults(
  defineProps<{
    label: string
    action?: string
    actionPending?: boolean
    tone?: 'neutral' | 'danger'
  }>(),
  {
    actionPending: false,
    tone: 'neutral',
  },
)

const emit = defineEmits<{
  action: []
}>()
</script>

<template>
  <div
    class="session-notice-bar flex w-full items-center gap-3 rounded-lg px-3 py-2 text-chrome"
    :class="
      tone === 'danger'
        ? 'bg-tint-danger text-on-danger'
        : 'bg-surface-raised text-fg-muted'
    "
    :role="tone === 'danger' ? 'alert' : 'status'"
  >
    <CircleX v-if="tone === 'danger'" :size="14" class="shrink-0" aria-hidden="true" />
    <span class="min-w-0 flex-1 break-words">{{ label }}</span>
    <Button
      v-if="action"
      size="sm"
      class="shrink-0"
      :loading="actionPending"
      @click="emit('action')"
      >{{ action }}</Button
    >
  </div>
</template>
