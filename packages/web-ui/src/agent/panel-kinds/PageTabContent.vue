<script setup lang="ts">
import { ref, watch } from 'vue'
import { ExternalLink } from '@lucide/vue'
import BrowserAddressBar from '../BrowserAddressBar.vue'
import IconButton from '../../ui/IconButton.vue'
import Tooltip from '../../ui/Tooltip.vue'
import { loadPageAddress, pageTabTitle, type PageTabData } from './page-data'

/**
 * A `page` tab's content (`web-application.md` § Work panel): an address bar
 * over a sandboxed frame in the user's own browser. The parent sees nothing of
 * a cross-origin page, so history stays unavailable and Refresh reloads the
 * tab's URL.
 */
const props = defineProps<{ tabId: string; data: PageTabData; shown: boolean }>()
const emit = defineEmits<{ update: [data: PageTabData] }>()

// Scripts, forms and popups work; popups land in ordinary browser tabs. Without
// allow-top-navigation the page cannot navigate the product away.
const SANDBOX = 'allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox allow-modals allow-downloads'
const HISTORY_UNAVAILABLE = 'A framed page keeps its history to itself'

// The draft is the viewer's typing, not the tab: it follows the tab's URL until edited.
const address = ref(props.data.url)
// Remounting the frame is the only reload the parent has over a cross-origin page.
const reloads = ref(0)
watch(
  () => [props.tabId, props.data.url],
  () => {
    address.value = props.data.url
    reloads.value = 0
  },
)

function submitAddress(): void {
  emit('update', loadPageAddress(props.data, address.value))
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <BrowserAddressBar
      :address="address"
      :history-reason="HISTORY_UNAVAILABLE"
      can-reload
      @update:address="address = $event"
      @submit="submitAddress"
      @reload="reloads += 1"
    >
      <template #trailing>
        <Tooltip content="Open in a browser tab" class="shrink-0">
          <a
            :href="data.url"
            target="_blank"
            rel="noopener noreferrer"
            class="flex"
            aria-label="Open in a browser tab"
          >
            <IconButton :icon="ExternalLink" variant="ghost" aria-hidden="true" />
          </a>
        </Tooltip>
      </template>
    </BrowserAddressBar>
    <iframe
      :key="`${tabId}:${data.url}:${reloads}`"
      :src="data.url"
      :title="pageTabTitle(data)"
      :sandbox="SANDBOX"
      referrerpolicy="no-referrer"
      class="min-h-0 w-full flex-1 border-0 border-t border-line bg-white"
    />
  </div>
</template>
