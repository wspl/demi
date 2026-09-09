<script setup lang="ts">
import Button from '../ui/Button.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import UserBlock from './blocks/UserBlock.vue'
import type { PendingSubmissionState } from './types'

defineProps<PendingSubmissionState>()
const emit = defineEmits<{ retry: [] }>()
</script>

<template>
  <div class="shrink-0 py-3" aria-live="polite">
    <UserBlock
      :content="text ? [{ type: 'text', text }] : []"
      :editable="false"
    />
    <p v-if="fileNames.length" class="mt-1 px-[var(--agent-pad-x,2rem)] text-right text-chrome text-fg-muted">
      {{ fileNames.join(', ') }}
    </p>
    <div
      class="mt-2 flex items-center justify-end gap-2 px-[var(--agent-pad-x,2rem)] text-chrome text-fg-muted"
    >
      <template v-if="sending"><IndeterminateSpinner /> Sending…</template>
      <template v-else-if="error">
        <span>{{ error }}</span>
        <Button @click="emit('retry')">Retry</Button>
      </template>
    </div>
  </div>
</template>
