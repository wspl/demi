<script setup lang="ts">
import { computed } from 'vue'
import { t } from '../infra/i18n'
import Button from '../ui/Button.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import { sessionStatusCopy, type SessionStatusKind } from './session-status'

const props = defineProps<{
  kind: SessionStatusKind
  /** A failed restore can name the reason under the label. */
  detail?: string
}>()

const emit = defineEmits<{
  retry: []
  create: []
}>()

const copy = computed(() => sessionStatusCopy(props.kind))
const busy = computed(() => props.kind === 'loading')
</script>

<template>
  <div
    class="flex h-full min-h-0 flex-1 flex-col items-center justify-center gap-2 px-6 py-10 text-center"
    :role="kind === 'failed' || kind === 'missing' ? 'alert' : 'status'"
    :aria-live="kind === 'failed' || kind === 'missing' ? 'assertive' : 'polite'"
    :aria-busy="busy || undefined"
  >
    <IndeterminateSpinner
      v-if="busy"
      :size="16"
      class="text-fg-subtle"
    />
    <p class="text-conversation text-fg-muted">{{ copy.label }}</p>
    <p
      v-if="kind === 'failed' && detail"
      class="max-w-sm text-[12px] leading-4 text-fg-subtle"
    >
      {{ detail }}
    </p>
    <Button
      v-if="copy.action === 'retry'"
      class="mt-1"
      @click="emit('retry')"
      >{{ t('agent.session.retry') }}</Button
    >
    <Button
      v-else-if="copy.action === 'create'"
      class="mt-1"
      @click="emit('create')"
      >{{ t('agent.session.new') }}</Button
    >
  </div>
</template>
