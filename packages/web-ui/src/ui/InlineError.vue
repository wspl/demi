<script setup lang="ts">
import { CloseLine } from '@mingcute/vue/close'

defineProps<{
  message: string
  dismissible?: boolean
}>()

const emit = defineEmits<{
  dismiss: []
}>()
</script>

<template>
  <div class="flex items-start gap-1.5">
    <p class="min-w-0 flex-1 text-[12px] leading-4 text-on-danger">{{ message }}</p>
    <div
      v-if="dismissible"
      class="flex shrink-0 cursor-pointer items-center justify-center rounded p-0.5 text-on-danger-muted transition-colors hover:bg-hover hover:text-on-danger"
      @click="emit('dismiss')"
    >
      <CloseLine :size="12" />
    </div>
  </div>
</template>
