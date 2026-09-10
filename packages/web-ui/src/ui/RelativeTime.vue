<script setup lang="ts">
import { computed } from 'vue'
import { useRelativeTime } from '../composables/useRelativeTime'

const props = defineProps<{ timestamp: string }>()
const label = useRelativeTime(() => props.timestamp)
const absoluteTime = computed(() => new Date(props.timestamp).toLocaleString())
</script>

<template>
  <time :datetime="timestamp" :title="absoluteTime">{{ label }}</time>
</template>
