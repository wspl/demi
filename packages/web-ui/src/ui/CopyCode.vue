<script setup lang="ts">
import { ref } from 'vue'
import { useClipboard } from '@vueuse/core'
import { Check, Copy } from '@lucide/vue'
import IconButton from './IconButton.vue'

const props = withDefaults(
  defineProps<{
    code: string;
    copyLabel?: string
  }>(),
  { copyLabel: 'Copy command' }
)
const { copy, copied } = useClipboard()
const copiedCode = ref('')
async function copyCode() {
  const value = props.code
  await copy(value)
  copiedCode.value = value
}
</script>

<template>
  <div
    class="flex min-w-0 items-start gap-2 rounded-md border border-line bg-surface px-3 py-2"
  >
    <code
      class="min-w-0 flex-1 select-text break-words font-mono text-[12px] leading-5 text-fg-body"
    >{{ code }}</code>
    <IconButton
      :icon="copied && copiedCode === code ? Check : Copy"
      size="sm"
      :aria-label="copied && copiedCode === code ? 'Copied' : copyLabel"
      @click="copyCode"
    />
  </div>
</template>
