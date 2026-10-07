<script setup lang="ts">
import { computed } from 'vue'

/**
 * Text with the parts a search matched marked: every occurrence of `query`,
 * case ignored, the characters at `indexes`, as a fuzzy match or a typed
 * prefix names them, or the pieces `ranges` names, as the backend's search
 * gives them.
 */
const props = defineProps<{
  text: string
  query?: string
  /** The matched characters' positions in `text`; given, `query` is not read. */
  indexes?: readonly number[]
  /** The matched pieces as `[start, end]` positions in `text`, end exclusive; given, `query` is not read. */
  ranges?: readonly (readonly number[])[]
}>()

interface Segment {
  text: string
  isMatch: boolean
}

/** Runs of matched and unmatched characters, from the positions that matched. */
function indexSegments(text: string, indexes: readonly number[]): Segment[] {
  const matched = new Set(indexes)
  const result: Segment[] = []
  for (let index = 0; index < text.length; index++) {
    const isMatch = matched.has(index)
    const last = result.at(-1)
    if (last && last.isMatch === isMatch) {
      last.text += text[index]
    } else {
      result.push({ text: text[index]!, isMatch })
    }
  }
  return result
}

/** Runs of matched and unmatched characters, from each occurrence of the query. */
function querySegments(text: string, query: string): Segment[] {
  const q = query.trim().toLowerCase()
  if (!q)
    return [{ text, isMatch: false }]

  const result: Segment[] = []
  const lower = text.toLowerCase()
  let cursor = 0

  while (cursor < text.length) {
    const matchIdx = lower.indexOf(q, cursor)
    if (matchIdx === -1) {
      result.push({ text: text.slice(cursor), isMatch: false })
      break
    }
    if (matchIdx > cursor) {
      result.push({ text: text.slice(cursor, matchIdx), isMatch: false })
    }
    result.push({ text: text.slice(matchIdx, matchIdx + q.length), isMatch: true })
    cursor = matchIdx + q.length
  }

  return result
}

/** Every position inside one of `ranges`. */
function rangeIndexes(ranges: readonly (readonly number[])[]): number[] {
  return ranges.flatMap(([start = 0, end = 0]) =>
    Array.from({ length: Math.max(0, end - start) }, (_, offset) => start + offset))
}

const segments = computed(() => {
  if (props.ranges)
    return indexSegments(props.text, rangeIndexes(props.ranges))
  return props.indexes
    ? indexSegments(props.text, props.indexes)
    : querySegments(props.text, props.query ?? '')
})
</script>

<template>
  <template v-for="(seg, i) in segments" :key="i">
    <mark v-if="seg.isMatch" class="bg-tint-highlight text-on-highlight">{{ seg.text }}</mark>
    <template v-else>{{ seg.text }}</template>
  </template>
</template>
