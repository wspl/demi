<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  text: string
  query: string
}>()

const segments = computed(() => {
  const q = props.query.trim().toLowerCase()
  if (!q) return [{ text: props.text, isMatch: false }]

  const result: { text: string; isMatch: boolean }[] = []
  const lower = props.text.toLowerCase()
  let cursor = 0

  while (cursor < props.text.length) {
    const matchIdx = lower.indexOf(q, cursor)
    if (matchIdx === -1) {
      result.push({ text: props.text.slice(cursor), isMatch: false })
      break
    }
    if (matchIdx > cursor) {
      result.push({ text: props.text.slice(cursor, matchIdx), isMatch: false })
    }
    result.push({ text: props.text.slice(matchIdx, matchIdx + q.length), isMatch: true })
    cursor = matchIdx + q.length
  }

  return result
})
</script>

<template>
  <template v-for="(seg, i) in segments" :key="i">
    <mark v-if="seg.isMatch" class="bg-tint-highlight text-on-highlight">{{ seg.text }}</mark>
    <template v-else>{{ seg.text }}</template>
  </template>
</template>
