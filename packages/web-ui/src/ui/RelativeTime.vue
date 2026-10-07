<script setup lang="ts">
import { computed } from 'vue'
import { formatMoment, useRelativeTime } from '../composables/useRelativeTime'

const props = defineProps<{ timestamp: string }>()
const label = useRelativeTime(() => props.timestamp)
const absoluteTime = computed(() => formatMoment(props.timestamp))
</script>

<template>
  <time :datetime="timestamp" :title="absoluteTime">{{ label }}</time>
</template>
