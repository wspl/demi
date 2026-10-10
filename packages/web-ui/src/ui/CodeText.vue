<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useResizeObserver } from '@vueuse/core'

/**
 * Code text, such as a command, that wraps at its spaces as an editor's word
 * wrap does: a word that does not fit moves to the next line whole, and only
 * a word longer than the whole line breaks, at any character, filling each
 * line to the edge. A word never breaks at a hyphen or a slash inside it, so
 * `echo` never shows as `ech` and `o`, nor `libatk-bridge2.0` as two names.
 * The text keeps its spaces and line breaks and copies exactly as given.
 *
 * The caller lays the element out as a block, such as a flex item: the
 * line's width decides which words are longer than a line.
 */
const props = defineProps<{
  text: string
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

interface Piece {
  text: string
  /** A run of characters other than white space; the rest is the space between. */
  word: boolean
  /** A word wider than a line, which alone may break inside. */
  long: boolean
}

const pieces = computed<Piece[]>(() =>
  props.text
    .split(/(\s+)/)
    .filter((text) => text !== '')
    .map((text) => {
      const word = !/^\s/.test(text)
      return {
        text,
        word,
        long: word && lineWidth.value > 0 && textWidth(text, font.value) > lineWidth.value,
      }
    }),
)

watch(pieces, () => emit('layout'), { flush: 'post' })

// The template keeps the words and the space between them on one line: white
// space between its tags would add to the text.
defineExpose({ element })
</script>

<script lang="ts">
let measuring: CanvasRenderingContext2D | null | undefined

/** How wide the text is drawn in the font, by a canvas shared by every instance. */
function textWidth(text: string, font: string): number {
  if (measuring === undefined) {
    measuring = document.createElement('canvas').getContext('2d')
  }
  // Without a 2D context nothing is measured, and every word stays whole.
  if (!measuring) {
    return 0
  }
  measuring.font = font
  return measuring.measureText(text).width
}
</script>

<template>
  <code ref="element" class="whitespace-pre-wrap"><template v-for="(piece, index) in pieces" :key="index"><span v-if="piece.word" :class="piece.long ? 'terminal-wrap' : 'whitespace-nowrap'">{{ piece.text }}</span><template v-else>{{ piece.text }}</template></template></code>
</template>
