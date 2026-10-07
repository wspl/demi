<script setup lang="ts">
import AsyncRegion from './AsyncRegion.vue'

/**
 * What the page shows where the app will be until it knows who is signed
 * in (`web-application.md` § Page synchronization): that it loads, or, while
 * it cannot reach the backend, as when the page is loaded during a restart,
 * that it connects; or the failure the backend answered the check with,
 * with Retry. The caller gives it its height.
 */
withDefaults(
  defineProps<{
    /** The page could not reach the backend so far. */
    connecting: boolean
    /** The backend's own failure of the check, in its words; null while there is none. */
    failure?: string | null
    /** Checks again, as `RegionStatus` takes it. */
    onRetry?: () => unknown
  }>(),
  { failure: null, onRetry: () => {} },
)
</script>

<template>
  <div class="flex items-center justify-center">
    <AsyncRegion
      :state="failure === null ? 'loading' : 'failed'"
      :label="connecting ? 'Connecting to Demi…' : 'Loading Demi…'"
      error="Couldn’t check your session."
      :detail="failure"
      :on-retry="onRetry"
    />
  </div>
</template>
