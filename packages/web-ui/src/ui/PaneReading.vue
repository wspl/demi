<script setup lang="ts">
import { t } from '../agent/locales/text'
import { usePaneWaiting } from '../agent/pane-ready'

/**
 * What a work panel pane's empty area says once it has waited long enough
 * (`agent/pane-ready.ts`): what it is reading, centred. It stands in the
 * area, which it covers, and shows over it though the area is hidden.
 */
const pane = usePaneWaiting()
</script>

<template>
  <div
    v-if="pane.waiting.value && pane.slow.value"
    role="status"
    class="visible absolute inset-0 z-10 flex select-none items-center justify-center px-6 text-center text-[13px] text-fg-faint"
    data-pane-reading
  >
    <span class="min-w-0 truncate">{{ t('panel.reading', { name: pane.name.value }) }}</span>
  </div>
</template>
