<script setup lang="ts">
import { ref, watch } from 'vue'
import { ArrowLeft, ArrowRight, ExternalLink, RotateCw } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import TextInput from '../ui/TextInput.vue'
import Tooltip from '../ui/Tooltip.vue'
import { loadBrowserAddress, type BrowserWorkTab } from './work-panel'

/**
 * One browser tab of the work panel (`web-application.md`): an address bar
 * over a sandboxed frame in the user's own browser. The parent sees nothing of
 * a cross-origin page, so history stays unavailable and Refresh reloads the
 * tab's URL.
 */
const props = defineProps<{ tab: BrowserWorkTab }>()
const emit = defineEmits<{ update: [tab: BrowserWorkTab] }>()

// Scripts, forms and popups work; popups land in ordinary browser tabs. Without
// allow-top-navigation the page cannot navigate the product away.
const SANDBOX = 'allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox allow-modals allow-downloads'
const HISTORY_UNAVAILABLE = 'A framed page keeps its history to itself'

// Remounting the frame is the only reload the parent has over a cross-origin page.
const reloads = ref(0)
watch(() => props.tab.id, () => {
  reloads.value = 0
})

function updateAddress(address: string): void {
  emit('update', { ...props.tab, address })
}

function submitAddress(): void {
  emit('update', loadBrowserAddress(props.tab))
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <div class="flex h-11 shrink-0 items-center gap-1 px-2">
      <div class="flex shrink-0 items-center">
        <IconButton :icon="ArrowLeft" variant="ghost" aria-label="Back" disabled :disabled-reason="HISTORY_UNAVAILABLE" />
        <IconButton :icon="ArrowRight" variant="ghost" aria-label="Forward" disabled :disabled-reason="HISTORY_UNAVAILABLE" />
        <IconButton
          :icon="RotateCw"
          variant="ghost"
          aria-label="Refresh"
          spin-on-click
          :disabled="tab.url === null"
          @click="reloads += 1"
        />
      </div>
      <TextInput
        class="ml-2 min-w-0 flex-1"
        :model-value="tab.address"
        placeholder="Enter address"
        aria-label="Browser address"
        @update:model-value="updateAddress"
        @keydown.enter="submitAddress"
      />
      <Tooltip v-if="tab.url !== null" content="Open in a browser tab" class="shrink-0">
        <a
          :href="tab.url"
          target="_blank"
          rel="noopener noreferrer"
          class="flex"
          aria-label="Open in a browser tab"
        >
          <IconButton :icon="ExternalLink" variant="ghost" aria-hidden="true" />
        </a>
      </Tooltip>
    </div>
    <iframe
      v-if="tab.url !== null"
      :key="`${tab.id}:${reloads}`"
      :src="tab.url"
      :title="tab.title"
      :sandbox="SANDBOX"
      referrerpolicy="no-referrer"
      class="min-h-0 w-full flex-1 border-0 border-t border-line bg-white"
    />
    <div
      v-else
      class="flex min-h-0 flex-1 items-center justify-center border-t border-line text-[13px] text-fg-faint"
    >
      Enter an address to open a page.
    </div>
  </div>
</template>
