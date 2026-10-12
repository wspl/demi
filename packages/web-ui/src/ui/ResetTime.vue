<script setup lang="ts">
import { computed } from 'vue'
import { formatMoment, useResetTime } from '../composables/useRelativeTime'

/** When a usage window resets, worded as every surface words it, following the clock while it shows. */
const props = defineProps<{ timestamp: string }>()
const label = useResetTime(() => props.timestamp)
const absoluteTime = computed(() => formatMoment(props.timestamp))
</script>

<template>
  <time :datetime="timestamp" :title="absoluteTime">{{ label }}</time>
</template>
