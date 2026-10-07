<script setup lang="ts">
import AsyncRegion from './AsyncRegion.vue'

/**
 * What the page shows where the app will be until it knows who is signed
 * in (`web-application.md` § Page synchronization): that it loads, or, while
 * it cannot reach the backend, as when the page is loaded during a restart,
 * that it connects. The caller gives it its height.
 */
defineProps<{
  /** The page could not reach the backend so far. */
  connecting: boolean
}>()
</script>

<template>
  <div class="flex items-center justify-center">
    <AsyncRegion state="loading" :label="connecting ? 'Connecting to Demi…' : 'Loading Demi…'" />
  </div>
</template>
