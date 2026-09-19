<script setup lang="ts">
import { getCurrentInstance, ref } from 'vue'
import { drawSelection, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from '@codemirror/view'
import { bracketMatching, foldGutter, foldKeymap } from '@codemirror/language'
import { defaultKeymap } from '@codemirror/commands'
import { highlightSelectionMatches } from '@codemirror/search'
import { indentationMarkers } from '@replit/codemirror-indentation-markers'
import { activeLineVisibilityExtension } from '../activeLineVisibility'
import { foldMarker } from '../foldMarker'
import { customScrollbarExtension } from '../scrollbars/extension'
import { searchExtension } from '../searchPanel'
import { stickyHeadersExtension } from '../sticky/extension'
import { useCodeView } from '../useCodeView'

/**
 * One file's text, read-only: syntax colors by the file's name, folding, the
 * enclosing blocks' first lines kept at the top while scrolling, and Mod-f to
 * find. Another file or text needs another editor.
 */
const props = defineProps<{
  /** The file's path, which selects its language. */
  path: string
  text: string
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
  stickyHeadersExtension(),
  searchExtension(getCurrentInstance()?.appContext ?? null),
  keymap.of([...defaultKeymap, ...foldKeymap]),
])
</script>

<template>
  <div ref="container" class="relative h-full w-full [&>.cm-editor]:h-full" />
</template>
