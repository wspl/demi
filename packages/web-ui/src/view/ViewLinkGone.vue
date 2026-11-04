<script setup lang="ts">
import { Link2Off } from '@lucide/vue'
import RegionStatus from '../ui/RegionStatus.vue'
import { t } from './locales/text'

/**
 * What a view link shows once it opens nothing: never made, expired or
 * removed, one answer for each, so it tells nothing of what exists.
 */
</script>

<template>
  <main class="flex h-full flex-col justify-center bg-surface" data-view-link-gone>
    <RegionStatus status="note" :icon="Link2Off" :label="t('view.gone')" />
  </main>
</template>
