<script setup lang="ts">
import Button from '../ui/Button.vue'

defineProps<{
  label: string
  action?: string
}>()

const emit = defineEmits<{
  action: []
}>()
</script>

<template>
  <div
    class="session-notice-bar flex w-full items-center justify-between rounded-lg bg-surface-raised p-3 text-chrome text-fg-muted"
    role="status"
  >
    <span>{{ label }}</span>
    <Button
      v-if="action"
      @click="emit('action')"
      >{{ action }}</Button
    >
  </div>
</template>
