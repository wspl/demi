<script setup lang="ts">
import { computed } from 'vue'
import { useResetTime } from '../composables/useRelativeTime'
import Tooltip from './Tooltip.vue'
import { formatMoment } from './dates'

/** When a usage window resets, worded as every surface words it, following the clock while it shows. */
const props = defineProps<{ timestamp: string }>()
const label = useResetTime(() => props.timestamp)
const absoluteTime = computed(() => formatMoment(props.timestamp))
</script>

<template>
  <Tooltip :content="absoluteTime">
    <time :datetime="timestamp">{{ label }}</time>
  </Tooltip>
</template>
