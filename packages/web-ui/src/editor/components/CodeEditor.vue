<script setup lang="ts">
import { getCurrentInstance, ref } from 'vue'
import type { StateEffect } from '@codemirror/state'
import { drawSelection, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from '@codemirror/view'
import { bracketMatching, foldGutter, foldKeymap } from '@codemirror/language'
import { defaultKeymap } from '@codemirror/commands'
import { highlightSelectionMatches } from '@codemirror/search'
import { indentationMarkers } from '@replit/codemirror-indentation-markers'
import { activeLineVisibilityExtension } from '../activeLineVisibility'
import { foldMarker } from '../foldMarker'
import { customScrollbarExtension } from '../scrollbars/extension'
import { searchExtension } from '../searchPanel'
import { useCodeView } from '../useCodeView'

/**
 * One file's text, read-only: syntax colors by the file's name, folding, and
 * Mod-f to find. A new text of the file replaces the old in place; another
 * file needs another editor, which can open where an earlier one of it was
 * scrolled to (`scrollTo`, from `left`).
 */
const props = defineProps<{
  /** The file's path, which selects its language. */
  path: string
  text: string
  /** Where it opens scrolled to: what an earlier editor of the file said as it went. */
  scrollTo?: StateEffect<unknown> | null
}>()

const emit = defineEmits<{
  /** The editor of the file at `path` goes, scrolled to this. */
  left: [path: string, snapshot: StateEffect<unknown>]
}>()

const container = ref<HTMLDivElement>()

useCodeView(container, props, [
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
], { scrollTo: props.scrollTo, left: (snapshot) => emit('left', props.path, snapshot) })
</script>

<template>
  <div ref="container" class="relative h-full w-full [&>.cm-editor]:h-full" />
</template>
