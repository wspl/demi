<script setup lang="ts">
import { t } from '../agent/locales/text'
import { usePaneWaiting } from '../agent/pane-ready'
import RegionStatus from './RegionStatus.vue'

/**
 * What a work panel pane's empty area says once it has waited long enough
 * (`agent/pane-ready.ts`): the loading pane, a spinner over what it is reading,
 * centred as every region's is. It stands in the area, which it covers, and
 * shows over it though the area is hidden.
 */
const pane = usePaneWaiting()
</script>

<template>
  <RegionStatus
    v-if="pane.waiting.value && pane.slow.value"
    class="visible absolute inset-0 z-10 select-none [&_p]:max-w-full [&_p]:truncate"
    status="loading"
    :label="t('panel.reading', { name: pane.name.value })"
    data-pane-reading
  />
</template>
