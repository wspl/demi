<script setup lang="ts">
import { provide } from 'vue'
import { deviceReportKey, type DeviceReport } from '@demicodes/web-ui/auth/device-unlock'

/**
 * Makes what is inside speak of another device than the one the gallery is
 * open on, as the browser of that system would report itself; with no report
 * it changes nothing.
 */
const props = defineProps<{ report?: DeviceReport }>()

// Without a report the offer speaks of the browser the gallery is open in.
if (props.report) {
  provide(deviceReportKey, props.report)
}
</script>

<template>
  <slot />
</template>
