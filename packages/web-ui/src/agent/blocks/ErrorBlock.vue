<script setup lang="ts">
import { computed, ref } from 'vue'
import { useClipboard } from '@vueuse/core'
import { RightSmallLine } from '@mingcute/vue/right-small'
import { CopyLine } from '@mingcute/vue/copy'
import { CheckLine } from '@mingcute/vue/check'
import { ArrowRightLine } from '@mingcute/vue/arrow-right'
import { t } from '@demicodes/web-ui/infra/i18n'

const props = defineProps<{
  message: string
  code?: string | null
  stack?: string
  /** Offer the Continue action. Only the conversation's tail error is
   * recoverable — older errors are records, not entry points. */
  recoverable: boolean
}>()

const emit = defineEmits<{
  continue: []
  retry: []
}>()

function handleRecovery() {
  emit('continue')
}

const isStackOpen = ref(false)

const copyableText = computed(() => {
  const parts = [props.message]
  if (props.code) parts.unshift(`[${props.code}]`)
  if (props.stack) parts.push(props.stack)
  return parts.join('\n')
})

const { copy, copied } = useClipboard({ copiedDuring: 1500 })
</script>

<template>
  <div class="overflow-hidden rounded-lg border border-on-danger-muted bg-tint-danger">
    <div class="flex items-center gap-2 px-3 py-2">
      <span class="text-[12px] font-medium uppercase tracking-wide text-on-danger">{{ t('agent.block.error') }}</span>
      <span
        v-if="props.code"
        class="rounded bg-tint-danger-strong px-1.5 py-0.5 font-mono text-[11px] text-on-danger-muted"
      >{{ props.code }}</span>
      <div class="flex-1" />
      <div
        class="flex cursor-pointer items-center rounded-md p-1 text-on-danger-muted transition-colors hover:bg-tint-danger hover:text-on-danger-muted"
        @click="copy(copyableText)"
      >
        <CheckLine v-if="copied" :size="13" class="text-on-success" />
        <CopyLine v-else :size="13" />
      </div>
    </div>
    <div class="border-t border-on-danger-muted px-3 py-2">
      <pre class="whitespace-pre-wrap break-words font-mono text-[12px] leading-5 text-on-danger">{{ props.message }}</pre>
    </div>
    <div v-if="props.stack">
      <div
        class="flex cursor-pointer items-center gap-1 border-t border-on-danger-muted px-3 py-1.5 text-[11px] text-on-danger-muted transition-colors hover:bg-tint-danger"
        @click="isStackOpen = !isStackOpen"
      >
        <RightSmallLine :size="14" class="transition-transform duration-150" :class="isStackOpen ? 'rotate-90' : ''" />
        <span class="uppercase tracking-wide">{{ t('agent.block.stackTrace') }}</span>
      </div>
      <div v-if="isStackOpen" class="border-t border-on-danger-muted px-3 py-2">
        <pre class="whitespace-pre-wrap break-words font-mono text-[11px] leading-4.5 text-on-danger-muted">{{ props.stack }}</pre>
      </div>
    </div>
    <div v-if="props.recoverable" class="flex items-center justify-end border-t border-on-danger-muted px-3 py-2">
      <div
        class="flex cursor-pointer items-center gap-1.5 rounded-md bg-tint-danger-strong px-2.5 py-1 text-[12px] font-medium text-on-danger transition-colors hover:bg-tint-danger-strong"
        @click="handleRecovery"
      >
        <ArrowRightLine :size="13" />
        {{ t('common.continue') }}
      </div>
    </div>
  </div>
</template>
