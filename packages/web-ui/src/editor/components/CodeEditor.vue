<script setup lang="ts">
import { getCurrentInstance, onBeforeUnmount, ref, watch, onMounted } from 'vue'
import { EditorView, drawSelection, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter, tooltips } from '@codemirror/view'
import { Compartment, EditorState, Text, type Extension } from '@codemirror/state'
import { indentOnInput, bracketMatching, foldCode, foldGutter, foldKeymap, toggleFold } from '@codemirror/language'
import { acceptCompletion, closeBrackets, closeBracketsKeymap, startCompletion } from '@codemirror/autocomplete'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { highlightSelectionMatches, searchKeymap } from '@codemirror/search'
import { searchExtension } from '../searchPanel'
import { activeLineVisibilityExtension, markActiveLineInteracted } from '../activeLineVisibility'
import { customScrollbarExtension } from '../scrollbars/extension'
import { stickyHeadersExtension } from '../sticky/extension'
import { wynkTheme, trackEditorView, untrackEditorView } from '../theme/cmTheme'
import { findLanguage } from '../language/findLanguage'
import { lspFileUri, lspDocSyncExtension, syncOpen } from '../lsp/docSync'
import { lspHoverExtension } from '../lsp/hover'
import { lspCompletionExtension } from '../lsp/completion'
import { lspDiagnosticsExtension } from '../lsp/diagnostics'
import { lspDefinitionExtension } from '../lsp/definition'
import { lspSemanticTokensExtension } from '../lsp/semanticTokens'
import { lspSignatureExtension } from '../lsp/signature'
import { lspInlayHintsExtension } from '../lsp/inlayHints'
import { lspFoldingExtension } from '../lsp/folding'
import { requestLspRefresh } from '../lsp/refresh'
import {
  fromLspPosition,
  getResourceBasename,
  toLspPosition,
} from '../lsp/utils'
import { indentationMarkers } from '@replit/codemirror-indentation-markers'
import type { EditorHost } from '../host/types'
import { VueRenderer } from '../render/vueRenderer'
import { flashElements, nextReplayToken } from '../navigation/flash'
import { centerRange, getCoveredLineNumbers, resetScrollTop } from '../navigation/reveal'
import FoldMarker from './editor/FoldMarker.vue'

interface FileSelection {
  start: { line: number; character: number }
  end?: { line: number; character: number }
}

const props = withDefaults(defineProps<{
  host: EditorHost
  readOnly?: boolean
}>(), {
  readOnly: false,
})

const emit = defineEmits<{
  change: []
  save: []
}>()

const appContext = getCurrentInstance()?.appContext ?? null
const containerRef = ref<HTMLDivElement>()
let view: EditorView | undefined
let isSuppressing = false
let currentResourceUri = ''
let lastFlashToken = 0

const langCompartment = new Compartment()
const readOnlyCompartment = new Compartment()

// Read-only here means a viewer: no edits and no caret, while the editor
// still owns the mouse, so a selection starts anywhere in it (gutter, past
// a line's end, below the last line) the way it does when editing.
const viewerTheme = EditorView.theme({
  '&.cm-viewer .cm-cursorLayer': { display: 'none' },
  '&.cm-viewer .cm-content': { caretColor: 'transparent' },
})
function readOnlyExtensions(readOnly: boolean) {
  return [
    EditorState.readOnly.of(readOnly),
    EditorView.editorAttributes.of({ class: readOnly ? 'cm-viewer' : '' }),
    viewerTheme,
  ]
}
const lspUriCompartment = new Compartment()

function buildExtensions(): Extension {
  return [
    drawSelection({ cursorBlinkRate: 1000 }),
    lineNumbers(),
    customScrollbarExtension(),
    stickyHeadersExtension(props.host),
    highlightActiveLine(),
    highlightActiveLineGutter(),
    activeLineVisibilityExtension(),
    history(),
    foldGutter({
      markerDOM(open) {
        const renderer = new VueRenderer(FoldMarker, { isOpen: open }, { appContext })
        return renderer.dom
      },
    }),
    indentOnInput(),
    indentationMarkers({ colors: { light: 'transparent', dark: 'var(--color-fg-ghost)', activeLight: 'transparent', activeDark: 'var(--color-fg-faint)' } }),
    bracketMatching(),
    closeBrackets(),
    highlightSelectionMatches(),
    searchExtension(props.host, appContext),
    tooltips({ parent: document.body }),
    keymap.of([
      ...closeBracketsKeymap,
      ...defaultKeymap,
      ...searchKeymap,
      ...historyKeymap,
      ...foldKeymap,
      indentWithTab,
      { key: 'Mod-s', run: () => { emit('save'); return true } },
    ]),
    EditorState.tabSize.of(2),
    wynkTheme(props.host),
    langCompartment.of([]),
    readOnlyCompartment.of(readOnlyExtensions(props.readOnly)),
    lspUriCompartment.of(lspFileUri.of(currentResourceUri)),
    lspDocSyncExtension(props.host),
    lspHoverExtension(props.host, appContext),
    lspCompletionExtension(props.host, appContext),
    lspDiagnosticsExtension(props.host, appContext),
    lspDefinitionExtension(props.host),
    lspSemanticTokensExtension(props.host),
    lspSignatureExtension(props.host, appContext),
    lspInlayHintsExtension(props.host),
    lspFoldingExtension(props.host),
    EditorView.updateListener.of((update) => {
      if (update.docChanged && !isSuppressing) emit('change')
    }),
  ]
}

onMounted(() => {
  if (!containerRef.value) return
  view = new EditorView({
    parent: containerRef.value,
    state: EditorState.create({ extensions: buildExtensions() }),
  })
  trackEditorView(view, props.host)
})

watch(() => props.readOnly, (value) => {
  view?.dispatch({ effects: readOnlyCompartment.reconfigure(readOnlyExtensions(value)) })
})

function setFile({ resourceUri, content, selection }: { resourceUri: string; content: string; selection?: FileSelection }) {
  if (!view) return

  currentResourceUri = resourceUri
  const filename = getResourceBasename(resourceUri)
  const langDesc = findLanguage(filename)

  const selectionSpec = selection
    ? (() => {
        const doc = Text.of(content.split('\n'))
        const anchor = fromLspPosition(doc, selection.start)
        const head = selection.end ? fromLspPosition(doc, selection.end) : anchor
        return { anchor, head }
      })()
    : undefined

  // Reset state to clear undo history when switching files
  view.setState(EditorState.create({
    doc: content,
    selection: selectionSpec,
    extensions: buildExtensions(),
  }))

  if (langDesc) {
    langDesc.load().then((lang) => {
      view?.dispatch({ effects: langCompartment.reconfigure(lang) })
    })
  }

  if (selectionSpec) {
    view.dispatch({ scrollIntoView: true })
  }

  // Notify LSP servers about the opened file
  syncOpen(props.host, resourceUri, content)
}

function getContent(): string {
  return view?.state.doc.toString() ?? ''
}

function resetContent(content: string) {
  if (!view) return
  isSuppressing = true
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: content },
  })
  isSuppressing = false
}

function setSelection(selection: FileSelection) {
  if (!view) return
  const anchor = fromLspPosition(view.state.doc, selection.start)
  const head = selection.end ? fromLspPosition(view.state.doc, selection.end) : anchor
  view.dispatch({
    selection: { anchor, head },
    scrollIntoView: true,
  })
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

function revealRange(selection: FileSelection) {
  if (!view) return
  centerRange(view, selection)
}

function flashRange(selection: FileSelection, options?: { durationMs?: number }) {
  if (!view) return
  const lineNumbers = getCoveredLineNumbers(view.state.doc, selection)
  if (lineNumbers.length === 0) return
  lastFlashToken = nextReplayToken(lastFlashToken)
  flashElements(getRenderedLineElements(lineNumbers), 'range', lastFlashToken, options?.durationMs ?? 800)
}

function revealAndFlashRange(selection: FileSelection, options?: { durationMs?: number }) {
  revealRange(selection)
  requestAnimationFrame(() => flashRange(selection, options))
}

function scrollToTop() {
  if (!view) return
  resetScrollTop(view)
}

function getSelection(): FileSelection | null {
  if (!view) return null
  const main = view.state.selection.main
  return {
    start: toLspPosition(view.state.doc, main.from),
    end: toLspPosition(view.state.doc, main.to),
  }
}

function insertText(text: string) {
  if (!view) return
  const main = view.state.selection.main
  view.dispatch({
    changes: { from: main.from, to: main.to, insert: text },
    selection: { anchor: main.from + text.length },
  })
}

function startCompletionAtSelection() {
  if (!view) return false
  view.focus()
  return startCompletion(view)
}

function acceptCurrentCompletion() {
  if (!view) return false
  view.focus()
  return acceptCompletion(view)
}

function foldAtSelection() {
  if (!view) return false
  return foldCode(view)
}

function toggleFoldAtSelection() {
  if (!view) return false
  return toggleFold(view)
}

function hoverSelection(selection: FileSelection) {
  if (!view) return false
  const pos = fromLspPosition(view.state.doc, selection.start)
  const coords = view.coordsAtPos(pos)
  if (!coords) return false
  const domAtPos = view.domAtPos(pos)
  const target = domAtPos.node.nodeType === Node.TEXT_NODE
    ? domAtPos.node.parentElement
    : domAtPos.node instanceof Element
      ? domAtPos.node
      : view.dom
  const clientX = coords.left + 2
  const clientY = Math.floor((coords.top + coords.bottom) / 2)
  view.focus()
  for (const type of ['mouseenter', 'mouseover', 'mousemove']) {
    target?.dispatchEvent(new MouseEvent(type, { bubbles: true, clientX, clientY }))
  }
  return true
}

function focus() {
  if (view) markActiveLineInteracted(view)
  view?.focus()
}

function refreshLspState() {
  if (!view) return
  requestLspRefresh(view)
}

onBeforeUnmount(() => {
  if (view) {
    untrackEditorView(view)
    view.destroy()
    view = undefined
  }
})

defineExpose({
  setFile,
  getContent,
  resetContent,
  setSelection,
  getSelection,
  revealRange,
  flashRange,
  revealAndFlashRange,
  scrollToTop,
  insertText,
  startCompletionAtSelection,
  acceptCurrentCompletion,
  foldAtSelection,
  hoverSelection,
  focus,
  startCompletion: startCompletionAtSelection,
  triggerSignatureHelp: () => !!view,
  toggleFold: toggleFoldAtSelection,
  refreshLspState,
})
</script>

<template>
  <div ref="containerRef" class="relative h-full w-full [&>.cm-editor]:h-full" />
</template>
