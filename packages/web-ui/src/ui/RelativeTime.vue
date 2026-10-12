<script setup lang="ts">
import { computed } from 'vue'
import { useRelativeTime } from '../composables/useRelativeTime'
import Tooltip from './Tooltip.vue'
import { formatMoment } from './dates'

const props = defineProps<{ timestamp: string }>()
const label = useRelativeTime(() => props.timestamp)
const absoluteTime = computed(() => formatMoment(props.timestamp))
</script>

<template>
  <Tooltip :content="absoluteTime">
    <time :datetime="timestamp">{{ label }}</time>
  </Tooltip>
</template>
