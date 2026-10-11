<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useResizeObserver } from '@vueuse/core'
import { textWidth } from './text-width'

/**
 * Code text, such as a command, that wraps at its spaces as an editor's word
 * wrap does: a word that does not fit moves to the next line whole, and only
 * a word longer than the whole line breaks, at any character, filling each
 * line to the edge. A word never breaks at a hyphen or a slash inside it, so
 * `echo` never shows as `ech` and `o`, nor `libatk-bridge2.0` as two names.
 * The text keeps its spaces and line breaks and copies exactly as given.
 *
 * The caller lays the element out as a block, such as a flex item: the
 * line's width decides which words are longer than a line. `runs` colors
 * runs of the text, as a highlighter gives them, without changing where it
 * wraps.
 */
const props = defineProps<{
  text: string
  /** CSS classes for runs of the text, by where each starts and ends in it; the rest keeps the element's color. */
  runs?: readonly { from: number; to: number; classes: string }[]
}>()

const emit = defineEmits<{
  /** The text was laid out anew, after its words or the line's width changed. */
  layout: []
}>()

const element = ref<HTMLElement>()
/** The width of a line, and the font words are measured in; 0 until laid out. */
const lineWidth = ref(0)
const font = ref('')

useResizeObserver(element, (entries) => {
  const entry = entries[0]
  if (!entry || !element.value) {
    return
  }
  const style = getComputedStyle(element.value)
  font.value = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`
  lineWidth.value = entry.contentRect.width
})

/** A part of a piece in one style: the classes of the run it falls in, or none. */
interface Segment {
  text: string
  classes: string | undefined
}

interface Piece {
  text: string
  /** A run of characters other than white space; the rest is the space between. */
  word: boolean
  /** A word wider than a line, which alone may break inside. */
  long: boolean
  segments: Segment[]
}

/** The text from `from` to `to`, cut where the colored runs start and end. */
function segmentsOf(from: number, to: number): Segment[] {
  const segments: Segment[] = []
  let at = from
  for (const run of props.runs ?? []) {
    if (run.to <= at || run.from >= to)
      continue
    if (run.from > at)
      segments.push({ text: props.text.slice(at, run.from), classes: undefined })
    const end = Math.min(run.to, to)
    segments.push({ text: props.text.slice(Math.max(run.from, at), end), classes: run.classes })
    at = end
  }
  if (at < to)
    segments.push({ text: props.text.slice(at, to), classes: undefined })
  return segments
}

const pieces = computed<Piece[]>(() => {
  let offset = 0
  return props.text
    .split(/(\s+)/)
    .filter((text) => text !== '')
    .map((text) => {
      const word = !/^\s/.test(text)
      const from = offset
      offset += text.length
      return {
        text,
        word,
        long: word && lineWidth.value > 0 && textWidth(text, font.value) > lineWidth.value,
        segments: segmentsOf(from, offset),
      }
    })
})

watch(pieces, () => emit('layout'), { flush: 'post' })

// The template keeps the words and the space between them on one line: white
// space between its tags would add to the text.
defineExpose({ element })
</script>

<template>
  <code ref="element" class="whitespace-pre-wrap"><template v-for="(piece, index) in pieces" :key="index"><span v-if="piece.word" :class="piece.long ? 'terminal-wrap' : 'whitespace-nowrap'"><template v-for="(segment, part) in piece.segments" :key="part"><span v-if="segment.classes" :class="segment.classes">{{ segment.text }}</span><template v-else>{{ segment.text }}</template></template></span><template v-else>{{ piece.text }}</template></template></code>
</template>
