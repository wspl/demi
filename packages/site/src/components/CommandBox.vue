<script setup lang="ts">
// The install command on a white card, with a button that copies it.
import { CheckIcon, CopyIcon } from '@lucide/vue'
import { onBeforeUnmount, ref } from 'vue'
import { INSTALL_COMMAND } from '@/links'

// The command around its address, which is colored.
const [before, url = '', after = ''] = INSTALL_COMMAND.split(/(https:\/\/\S+)/)

const copied = ref(false)
let reset: ReturnType<typeof setTimeout> | undefined

async function copy(): Promise<void> {
  await navigator.clipboard.writeText(INSTALL_COMMAND)
  copied.value = true
  clearTimeout(reset)
  reset = setTimeout(() => {
    copied.value = false
  }, 1600)
}

onBeforeUnmount(() => clearTimeout(reset))
</script>

<template>
  <div class="command">
    <span class="prompt" aria-hidden="true">$</span>
    <code>{{ before }}<span class="url">{{ url }}</span>{{ after }}</code>
    <button
      class="copy"
      type="button"
      :aria-label="copied ? 'Copied' : 'Copy the command'"
      @click="copy"
    >
      <CheckIcon v-if="copied" :size="16" />
      <CopyIcon v-else :size="16" />
    </button>
  </div>
</template>

<style scoped>
.command {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  max-width: 100%;
  padding: 0.625rem 0.625rem 0.625rem 1.125rem;
  border: 1px solid var(--line);
  border-radius: 0.875rem;
  background: var(--paper);
  box-shadow: var(--shadow-card);
  font-family: var(--mono);
  font-size: 0.875rem;
}

.prompt {
  color: var(--ink-faint);
  user-select: none;
}

/* A box too narrow for the whole command cuts it off; it still copies
   whole. */
code {
  flex: 1;
  overflow-x: auto;
  font-family: inherit;
  white-space: nowrap;
  scrollbar-width: none;
}

.url {
  color: var(--accent);
}

.copy {
  display: grid;
  flex: none;
  place-items: center;
  width: 2.125rem;
  height: 2.125rem;
  border: 1px solid var(--line);
  border-radius: 0.5625rem;
  background: var(--cream);
  color: var(--ink-soft);
  cursor: pointer;
  transition: background-color 0.15s, color 0.15s;
}

.copy:hover {
  background: #f3ece2;
  color: var(--ink);
}
</style>
