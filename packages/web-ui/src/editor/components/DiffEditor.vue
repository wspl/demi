<script setup lang="ts">
import { getCurrentInstance, reactive, ref, toRef } from 'vue'
import { drawSelection, EditorView, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from '@codemirror/view'
import { defaultKeymap } from '@codemirror/commands'
import { activeLineVisibilityExtension } from '../activeLineVisibility'
import { customScrollbarExtension } from '../scrollbars/extension'
import { searchExtension } from '../searchPanel'
import { unifiedDiff } from '../unifiedDiff'
import { useCodeView } from '../useCodeView'

/**
 * Two versions of one file as a unified diff, read-only: the current text,
 * with each removed stretch shown above what replaced it, and the unchanged
 * stretches between changes folded but for three lines around each change
 * (`file-previews.md` § Changes, `unifiedDiff`). Focused, it keeps a caret
 * that the arrows move, Shift extends a selection from and Mod-f finds from
 * (`file-previews.md` § Code). New texts of the file
 * replace the old in place, keeping the place the user is looking at;
 * another file needs another editor.
 */
const props = defineProps<{
  original: string
  modified: string
  /** The file's path, which selects its language. */
  path: string
}>()

const diffTheme = EditorView.theme({
  '.cm-lineNumbers .cm-gutterElement': {
    paddingRight: '8px',
  },
  // The note after an added last line without a newline belongs to the added
  // lines. An added line keeps its color as the active line, whose own the
  // editor's theme gives it after this.
  '&.cm-diff .cm-line.cm-changedLine, &.cm-diff .cm-content > .cm-noNewline': {
    backgroundColor: 'color-mix(in srgb, var(--color-on-success) 15%, transparent)',
  },
  // A real background on inserted text, so it matches the deleted stretches.
  '&.cm-diff .cm-changedText': {
    background: 'color-mix(in srgb, var(--color-on-success) 22%, transparent)',
    borderRadius: '2px',
    padding: '1px 0',
    boxDecorationBreak: 'clone',
    WebkitBoxDecorationBreak: 'clone',
  },
  '&.cm-diff .cm-deletedText': { background: '#ff000033' },
  '&.cm-diff .cm-deletedChunk': { backgroundColor: 'color-mix(in srgb, var(--color-on-danger) 12%, transparent)', opacity: '0.6' },
})

const container = ref<HTMLDivElement>()
const appContext = getCurrentInstance()?.appContext ?? null

// The view is built once its language loads: by then the original may have
// been read again, and the diff compares against the one it has then.
useCodeView(container, reactive({
  path: props.path,
  text: toRef(props, 'modified'),
  original: toRef(props, 'original'),
}), () => [
  diffTheme,
  drawSelection(),
  lineNumbers(),
  highlightActiveLine(),
  highlightActiveLineGutter(),
  activeLineVisibilityExtension(),
  customScrollbarExtension(),
  searchExtension(appContext),
  keymap.of(defaultKeymap),
  unifiedDiff(props.original),
])
</script>

<template>
  <div ref="container" class="h-full w-full [&>.cm-editor]:h-full" />
</template>
