<script setup lang="ts">
import { getCurrentInstance, ref, watch } from 'vue'
import type { StateEffect } from '@codemirror/state'
import { drawSelection, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from '@codemirror/view'
import { bracketMatching, foldGutter, foldKeymap } from '@codemirror/language'
import { defaultKeymap } from '@codemirror/commands'
import { highlightSelectionMatches } from '@codemirror/search'
import { indentationMarkers } from '@replit/codemirror-indentation-markers'
import { activeLineVisibilityExtension } from '../activeLineVisibility'
import { foldMarker } from '../foldMarker'
import { foundLineExtension, revealLine } from '../foundLine'
import { customScrollbarExtension } from '../scrollbars/extension'
import { searchExtension } from '../searchPanel'
import { useCodeView } from '../useCodeView'

/**
 * One file's text, read-only: syntax colors by the file's name, folding, and
 * Mod-f to find. A new text of the file replaces the old in place. Another file
 * needs another editor.
 *
 * A new editor can open where an earlier one of the file was scrolled to
 * (`scrollTo`, from `left`), or at a line (`line`), scrolled there and marked for
 * a moment.
 */
const props = defineProps<{
  /** The file's path, which selects its language. */
  path: string
  text: string
  /** Where it opens scrolled to: what an earlier editor of the file said as it went. */
  scrollTo?: StateEffect<unknown> | null
  /** The line to show, 1-based: as the editor is built, and again each time it changes. */
  line?: number | null
}>()

const emit = defineEmits<{
  /** The editor of the file at `path` goes, scrolled to this. */
  left: [path: string, snapshot: StateEffect<unknown>]
  /** The editor is built and shows the file's text, once its language is in. */
  built: [path: string]
}>()

const container = ref<HTMLDivElement>()

const view = useCodeView(container, props, [
  drawSelection(),
  lineNumbers(),
  foldGutter({ markerDOM: foldMarker }),
  highlightActiveLine(),
  highlightActiveLineGutter(),
  activeLineVisibilityExtension(),
  indentationMarkers({ colors: { light: 'transparent', dark: 'var(--color-fg-ghost)', activeLight: 'transparent', activeDark: 'var(--color-fg-faint)' } }),
  bracketMatching(),
  highlightSelectionMatches(),
  customScrollbarExtension(),
  searchExtension(getCurrentInstance()?.appContext ?? null),
  keymap.of([...defaultKeymap, ...foldKeymap]),
  foundLineExtension(),
], { scrollTo: props.scrollTo, left: (snapshot) => emit('left', props.path, snapshot) })

watch(view, (built) => {
  if (built)
    emit('built', props.path)
})

watch([view, () => props.line], ([built, line]) => {
  if (built && line)
    revealLine(built, line)
})
</script>

<template>
  <div ref="container" class="relative h-full w-full [&>.cm-editor]:h-full" />
</template>
