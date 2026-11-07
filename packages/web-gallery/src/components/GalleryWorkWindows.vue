<script setup lang="ts">
import { watch } from 'vue'
import SessionWindows from '@demicodes/web-ui/windows/SessionWindows.vue'
import { useWindowStage } from '@demicodes/web-ui/windows/stage'
import { gallerySessionWork } from '../fixtures/session-work'
import type { GalleryWork } from '../fixtures/work-panel'

/**
 * The product's windows of a conversation's work over one specimen's state,
 * with its taskbar, every event acting on it as the product's store would.
 * The conversation they stand over is a placeholder: the specimen shows the
 * windows. A window that opens fills the box, as a maximized one does, so the
 * view it holds shows at the box's size; Restore floats it.
 */
const props = defineProps<{ work: GalleryWork; beforeFirstMessage?: boolean; deviceOffline?: string | null }>()

const sessionWork = gallerySessionWork(props.work, {
  beforeFirstMessage: () => props.beforeFirstMessage ?? false,
  deviceOffline: () => props.deviceOffline ?? null,
})
const stage = useWindowStage()
const seen = new Set<string>()
watch(() => stage.state.windows.map((window) => window.id), (ids) => {
  for (const id of ids.filter((each) => !seen.has(each))) {
    seen.add(id)
    if (stage.find(id)?.placement !== 'minimized') {
      stage.maximize(id)
    }
  }
}, { immediate: true })
</script>

<template>
  <div class="h-full min-w-0 flex-1" data-region>
    <SessionWindows :stage="stage" :work="sessionWork" :own="[]" :bottom="0">
      <div class="grid h-full place-items-center text-[13px] text-fg-faint">Conversation</div>
    </SessionWindows>
  </div>
</template>
