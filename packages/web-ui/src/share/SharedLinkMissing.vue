<script setup lang="ts">
import { Link2Off } from '@lucide/vue'
import RegionStatus from '../ui/RegionStatus.vue'
import { t } from './locales/text'

/** What a shared link shows when it names no share: never made, or removed by Stop Sharing. */
</script>

<template>
  <main class="flex h-full flex-col bg-surface" data-shared-link-missing>
    <div class="flex min-h-0 flex-1 flex-col justify-center">
      <RegionStatus status="note" :icon="Link2Off" :label="t('share.missing')" />
    </div>
    <footer class="shrink-0 py-2 text-center text-detail text-fg-subtle">{{ t('share.footer') }}</footer>
  </main>
</template>
