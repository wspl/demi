<script setup lang="ts">
import { ref } from 'vue'
import { useClipboard } from '@vueuse/core'
import { Check, Copy } from '@lucide/vue'
import CodeText from './CodeText.vue'
import IconButton from './IconButton.vue'

const props = withDefaults(
  defineProps<{
    code: string;
    copyLabel?: string
  }>(),
  { copyLabel: 'Copy' }
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
  <div class="copy-code flex min-w-0 items-start rounded-md border border-line bg-surface">
    <CodeText
      :text="code"
      class="copy-code-text min-w-0 flex-1 select-text font-mono text-[12px] text-fg-body"
    />
    <span class="copy-code-action">
      <IconButton
        :icon="copied && copiedCode === code ? Check : Copy"
        size="sm"
        :aria-label="copied && copiedCode === code ? 'Copied' : copyLabel"
        @click="copyCode"
      />
    </span>
  </div>
</template>

<style scoped>
/* The box is as high as one line of code and its padding. Copy sits at its
   top-right corner as far from the top and right edges as centering puts it
   in a one-line box: (box height - control height) / 2; a wrapped command
   grows downward and leaves Copy where it was. */
.copy-code {
  --copy-code-line: 1.25rem;
  --copy-code-pad-y: 0.5rem;
  --copy-code-inset: calc((var(--copy-code-line) + 2 * var(--copy-code-pad-y) - var(--spacing-hit-sm)) / 2);
}

.copy-code-text {
  line-height: var(--copy-code-line);
  padding: var(--copy-code-pad-y) 0 var(--copy-code-pad-y) 0.75rem;
}

.copy-code-action {
  display: flex;
  flex-shrink: 0;
  line-height: 0;
  padding: var(--copy-code-inset);
}
</style>
