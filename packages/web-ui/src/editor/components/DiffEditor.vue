<script setup lang="ts">
import { getCurrentInstance, onBeforeUnmount, ref, onMounted } from 'vue'
import { EditorView, ViewPlugin, lineNumbers, highlightActiveLine, highlightActiveLineGutter, tooltips } from '@codemirror/view'
import { EditorState, Text } from '@codemirror/state'
import { getChunks, unifiedMergeView } from '@codemirror/merge'
import { activeLineVisibilityExtension } from '../activeLineVisibility'
import { customScrollbarExtension, scrollbarDiffOriginalDoc } from '../scrollbars/extension'
import { requestLspRefresh } from '../lsp/refresh'
import { wynkTheme, trackEditorView, untrackEditorView } from '../theme/cmTheme'
import { findLanguage } from '../language/findLanguage'
import { lspFileUri, lspDocSyncExtension, syncOpen } from '../lsp/docSync'
import { lspHoverExtension } from '../lsp/hover'
import { lspDiagnosticsExtension } from '../lsp/diagnostics'
import { lspSemanticTokensExtension } from '../lsp/semanticTokens'
import { lspInlayHintsExtension } from '../lsp/inlayHints'
import { lspDefinitionExtension } from '../lsp/definition'
import { lspFoldingExtension } from '../lsp/folding'
import { flashElements, nextReplayToken } from '../navigation/flash'
import { normalizeDiffChunks } from '../navigation/diffChunks'
import { centerLine, resetScrollTop } from '../navigation/reveal'
import type { EditorHost } from '../host/types'

const props = withDefaults(defineProps<{
  host: EditorHost
  original: string
  modified: string
  filename?: string
  resourceUri?: string
  /** Fold the unchanged stretches between changes behind an "N lines folded" line; off shows the whole file. */
  collapseUnchanged?: boolean
}>(), {
  filename: '',
  resourceUri: '',
  collapseUnchanged: false,
})

const containerRef = ref<HTMLDivElement>()
const appContext = getCurrentInstance()?.appContext ?? null
let view: EditorView | undefined
let lastFlashToken = 0

onMounted(async () => {
  if (!containerRef.value) return

  const langDesc = findLanguage(props.filename)
  const langExt = langDesc ? [await langDesc.load()] : []

  view = new EditorView({
    parent: containerRef.value,
    state: EditorState.create({
      doc: props.modified,
      extensions: [
        wynkTheme(props.host),
        customScrollbarExtension(),
        scrollbarDiffOriginalDoc.of(Text.of(props.original.split('\n'))),
        lineNumbers(),
        highlightActiveLine(),
        highlightActiveLineGutter(),
        activeLineVisibilityExtension(),
        EditorState.readOnly.of(true),
        EditorState.tabSize.of(2),
        unifiedMergeView({
          original: props.original,
          highlightChanges: true,
          gutter: false,
          mergeControls: false,
          syntaxHighlightDeletions: true,
          ...(props.collapseUnchanged ? { collapseUnchanged: { margin: 5, minSize: 6 } } : {}),
        }),
        // The folded stretches read "N lines folded" rather than the merge view's own wording.
        ...(props.collapseUnchanged ? [ViewPlugin.define((view) => {
          function relabel() {
            for (const el of view.dom.querySelectorAll('.cm-collapsedLines')) {
              if (el.textContent?.includes('folded')) continue
              const match = el.textContent?.match(/(\d+)/)
              if (!match?.[1]) continue
              el.textContent = `${match[1]} lines folded`
            }
          }
          relabel()
          return { update: relabel }
        })] : []),
        ...(props.resourceUri ? [
          lspFileUri.of(props.resourceUri),
          lspDocSyncExtension(props.host),
          lspHoverExtension(props.host, appContext),
          lspDiagnosticsExtension(props.host, appContext),
          lspSemanticTokensExtension(props.host),
          lspInlayHintsExtension(props.host),
          lspDefinitionExtension(props.host),
          lspFoldingExtension(props.host),
        ] : []),
        ...langExt,
        tooltips({ parent: document.body }),
        EditorView.theme({
          '.cm-lineNumbers .cm-gutterElement': {
            paddingRight: '8px',
          },
          '&.cm-merge-b .cm-changedLine': { backgroundColor: 'color-mix(in srgb, var(--color-on-success) 15%, transparent)' },
          // Override CodeMirror merge's default green bottom-line gradient with a
          // real background highlight so inserted text matches deleted chunks.
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
          '.cm-collapsedLines': {
            display: 'flex',
            alignItems: 'center',
            gap: '12px',
            padding: '12px 16px',
            fontSize: '12px',
            fontWeight: '500',
            color: 'var(--color-fg)',
            background: 'color-mix(in srgb, var(--color-fg-subtle) 8%, transparent)',
            cursor: 'pointer',
            '&:before, &:after': {
              content: '""',
              flex: '1',
              height: '4px',
              backgroundImage: `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='8' height='4'%3E%3Cpath d='M0 2 Q2 0 4 2 Q6 4 8 2' fill='none' stroke='%23888' stroke-width='0.8'/%3E%3C/svg%3E")`,
              backgroundRepeat: 'repeat-x',
              margin: '0',
            },
            '&:hover': {
              background: 'color-mix(in srgb, var(--color-fg-subtle) 15%, transparent)',
              color: 'var(--color-fg)',
            },
          },
        }),
      ],
    }),
  })

  if (props.resourceUri) {
    syncOpen(props.host, props.resourceUri, props.modified)
  }

  trackEditorView(view, props.host)
})

function refreshLspState() {
  if (!view) return
  requestLspRefresh(view)
}

function getDiffChunks() {
  if (!view) return []
  const chunkState = getChunks(view.state)
  const originalDoc = view.state.facet(scrollbarDiffOriginalDoc)
  if (!chunkState || !originalDoc) return []
  return normalizeDiffChunks(chunkState.chunks, originalDoc, view.state.doc)
}

function getRenderedLineElements(lineNumbers: number[]) {
  if (!view) return []
  const lines = new Set<HTMLElement>()
  for (const lineNumber of lineNumbers) {
    const line = view.state.doc.line(Math.max(1, Math.min(lineNumber, view.state.doc.lines)))
    const domAtPos = view.domAtPos(line.from)
    const target = domAtPos.node.nodeType === Node.TEXT_NODE
      ? domAtPos.node.parentElement
      : domAtPos.node instanceof Element
        ? domAtPos.node
        : null
    const lineEl = target?.closest<HTMLElement>('.cm-line')
    if (lineEl) lines.add(lineEl)
  }
  return [...lines]
}

function listDiffChunks() {
  return getDiffChunks()
}

function revealDiffChunk(chunkRef: { id: string }) {
  if (!view) return
  const chunk = getDiffChunks().find((item) => item.id === chunkRef.id)
  if (!chunk) return
  centerLine(view, chunk.anchorLineNumber)
}

function flashDiffChunk(chunkRef: { id: string }, options?: { durationMs?: number }) {
  if (!view) return
  const chunk = getDiffChunks().find((item) => item.id === chunkRef.id)
  if (!chunk) return
  lastFlashToken = nextReplayToken(lastFlashToken)
  flashElements(getRenderedLineElements(chunk.changedLineNumbers), 'diff-changed', lastFlashToken, options?.durationMs ?? 800)
  if (chunk.deletedBlockIndex !== undefined) {
    const deletedBlock = [...view.dom.querySelectorAll<HTMLElement>('.cm-deletedChunk')][chunk.deletedBlockIndex]
    if (deletedBlock) {
      flashElements([deletedBlock], 'diff-deleted', lastFlashToken, options?.durationMs ?? 800)
    }
  }
}

function revealAndFlashDiffChunk(chunkRef: { id: string }, options?: { durationMs?: number }) {
  revealDiffChunk(chunkRef)
  requestAnimationFrame(() => flashDiffChunk(chunkRef, options))
}

function scrollToTop() {
  if (!view) return
  resetScrollTop(view)
}

onBeforeUnmount(() => {
  if (view) {
    untrackEditorView(view)
    view.destroy()
    view = undefined
  }
})

defineExpose({
  refreshLspState,
  listDiffChunks,
  revealDiffChunk,
  flashDiffChunk,
  revealAndFlashDiffChunk,
  scrollToTop,
})
</script>

<template>
  <div ref="containerRef" class="h-full w-full [&>.cm-editor]:h-full" />
</template>
