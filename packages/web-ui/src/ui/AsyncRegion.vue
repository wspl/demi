<script setup lang="ts">
import Button from './Button.vue'
import IndeterminateSpinner from './IndeterminateSpinner.vue'

withDefaults(
  defineProps<{
    state?: 'loading' | 'ready' | 'failed'
    label?: string
    error?: string
  }>(),
  {
    state: 'ready',
    label: 'Loading…',
    error: 'Could not load this content.',
  },
)
defineEmits<{ retry: [] }>()
</script>

<template>
  <slot v-if="state === 'ready'" />
  <div
    v-else
    class="flex min-h-24 items-center justify-center gap-2 px-4 py-6 text-[13px] text-fg-subtle"
    role="status"
    :aria-busy="state === 'loading'"
  >
    <template v-if="state === 'loading'">
      <IndeterminateSpinner :size="14" />
      <span>{{ label }}</span>
    </template>
    <template v-else>
      <span>{{ error }}</span>
      <Button size="sm" @click="$emit('retry')">Retry</Button>
    </template>
  </div>
</template>
