<script setup lang="ts">
import { ArrowRightLine } from '@mingcute/vue/arrow-right'
import { t } from '@demicodes/web-ui/infra/i18n'

const props = defineProps<{
  /** Offer the Continue action; see ErrorBlock — only the tail block recovers. */
  recoverable: boolean
}>()

const emit = defineEmits<{
  continue: []
  retry: []
}>()

function handleRecovery() {
  emit('continue')
}
</script>

<template>
  <div class="flex h-6 items-center justify-between">
      <span class="rounded bg-fg-ghost/60 px-1.5 py-0.5 text-[11px] text-fg-subtle">{{ t('agent.block.aborted') }}</span>
      <div
        v-if="props.recoverable"
        class="flex cursor-pointer items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] text-fg-subtle transition-colors hover:bg-active hover:text-fg-body"
        @click="handleRecovery"
      >
        <ArrowRightLine :size="12" />
        {{ t('common.continue') }}
      </div>
  </div>
</template>
