<script setup lang="ts">
import { reactive, ref, toRef } from 'vue'
import { EditorView, highlightActiveLine, highlightActiveLineGutter, lineNumbers } from '@codemirror/view'
import { unifiedMergeView } from '@codemirror/merge'
import { activeLineVisibilityExtension } from '../activeLineVisibility'
import { customScrollbarExtension } from '../scrollbars/extension'
import { useCodeView } from '../useCodeView'

/**
 * Two versions of one file as a unified diff, read-only: the current text,
 * with each removed stretch shown above what replaced it, and the unchanged
 * stretches between changes folded but for three lines around each change
 * (`file-previews.md` § Changes). New texts of the file replace the old in
 * place; another file needs another editor.
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
  '&.cm-merge-b .cm-changedLine': { backgroundColor: 'color-mix(in srgb, var(--color-on-success) 15%, transparent)' },
  // A real background on inserted text, in place of the merge view's green
  // underline, so it matches the deleted stretches.
  '&.cm-merge-b .cm-changedText': {
    background: 'color-mix(in srgb, var(--color-on-success) 22%, transparent)',
    borderRadius: '2px',
    padding: '1px 0',
    boxDecorationBreak: 'clone',
    WebkitBoxDecorationBreak: 'clone',
  },
  '&.cm-merge-b .cm-insertedLine, &.cm-merge-b ins.cm-insertedLine': {
    textDecoration: 'none !important',
  },
  '.cm-deletedChunk': { backgroundColor: 'color-mix(in srgb, var(--color-on-danger) 12%, transparent)', opacity: '0.6' },
})

const container = ref<HTMLDivElement>()

useCodeView(container, reactive({
  path: props.path,
  text: toRef(props, 'modified'),
  original: toRef(props, 'original'),
}), [
  diffTheme,
  lineNumbers(),
  highlightActiveLine(),
  highlightActiveLineGutter(),
  activeLineVisibilityExtension(),
  customScrollbarExtension(),
  unifiedMergeView({
    original: props.original,
    highlightChanges: true,
    gutter: false,
    mergeControls: false,
    syntaxHighlightDeletions: true,
    // Three lines of context, as GitHub and git show; a stretch of four or
    // more beyond that folds into one line a click unfolds.
    collapseUnchanged: { margin: 3, minSize: 4 },
  }),
])
</script>

<template>
  <div ref="container" class="h-full w-full [&>.cm-editor]:h-full" />
</template>
