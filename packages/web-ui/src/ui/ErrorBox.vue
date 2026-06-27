<script setup lang="ts">
import { useClipboard } from '@vueuse/core'
import { CheckLine } from '@mingcute/vue/check'
import { CopyLine } from '@mingcute/vue/copy'
import { CloseLine } from '@mingcute/vue/close'

const props = defineProps<{
  message: string
  copyable?: boolean
  dismissible?: boolean
}>()

const emit = defineEmits<{
  dismiss: []
}>()

const { copy, copied } = useClipboard({ copiedDuring: 1500 })
</script>

<template>
  <div class="rounded-lg border border-on-danger-muted bg-tint-danger px-3 py-2">
    <div class="flex items-start gap-2">
      <pre class="m-0 min-w-0 flex-1 whitespace-pre-wrap break-words font-mono text-[12px] leading-5 text-on-danger">{{ message }}</pre>
      <div v-if="copyable || dismissible" class="flex shrink-0 items-center gap-0.5">
        <span
          v-if="copyable"
          class="flex cursor-pointer items-center rounded-md p-1 text-on-danger-muted transition-colors hover:bg-tint-danger-strong hover:text-on-danger"
          @click="copy(props.message)"
        >
          <CheckLine v-if="copied" :size="12" class="text-on-success" />
          <CopyLine v-else :size="12" />
        </span>
        <div
          v-if="dismissible"
          class="flex cursor-pointer items-center justify-center rounded p-1 text-on-danger-muted transition-colors hover:bg-tint-danger-strong hover:text-on-danger"
          @click="emit('dismiss')"
        >
          <CloseLine :size="12" />
        </div>
      </div>
    </div>
  </div>
</template>
