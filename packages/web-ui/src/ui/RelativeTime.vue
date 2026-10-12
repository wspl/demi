<script setup lang="ts">
import { computed } from 'vue'
import { useRelativeTime } from '../composables/useRelativeTime'
import { formatMoment } from './dates'

const props = defineProps<{ timestamp: string }>()
const label = useRelativeTime(() => props.timestamp)
const absoluteTime = computed(() => formatMoment(props.timestamp))
</script>

<template>
  <time :datetime="timestamp" :title="absoluteTime">{{ label }}</time>
</template>
