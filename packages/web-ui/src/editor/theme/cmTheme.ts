import { Compartment } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { tags } from '@lezer/highlight'
import { codeThemes, type CodeThemePalette } from './codeThemes'
import type { EditorHost } from '../host/types'

function buildEditorTheme(p: CodeThemePalette, isDark: boolean) {
  return EditorView.theme({
    '&': {
      color: p.foreground,
      backgroundColor: 'var(--color-surface-editor)',
      border: 'none',
      '--code-variable': p.variable,
      '--code-constant': p.constant,
      '--code-function': p.function,
      '--code-type': p.type,
    },
    '&.cm-editor.cm-focused': { outline: 'none' },
    '.cm-content': {
      caretColor: p.cursor,
      fontFamily: 'inherit',
      padding: '12px 0 0',
    },
    '.cm-scroller': {
      fontSize: '13px',
      lineHeight: '20px',
      scrollbarWidth: 'none',
      boxSizing: 'border-box',
    },
    '.cm-scroller::-webkit-scrollbar': { display: 'none' },
    '.cm-cursor, .cm-dropCursor': { borderLeftColor: p.cursor, borderLeftWidth: '2px' },
    '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection': {
      backgroundColor: p.selection,
    },
    '.cm-selectionBackground': { borderRadius: '2px' },
    '.cm-activeLine': { backgroundColor: isDark ? '#ffffff0a' : '#00000008' },
    '.cm-activeLineGutter': { backgroundColor: 'transparent', color: p.activeLineNumber },
    '&:not(.cm-user-interacted) .cm-activeLine': { backgroundColor: 'transparent' },
    '&:not(.cm-user-interacted) .cm-activeLineGutter': { color: p.lineNumber },
    '.cm-gutters': { backgroundColor: 'var(--color-surface)', color: p.lineNumber, border: 'none' },
    '.cm-lineNumbers .cm-gutterElement': { paddingLeft: '12px' },
    '.cm-tooltip': { backgroundColor: 'var(--color-surface-raised)', border: '1px solid var(--color-line)' },
    '.cm-tooltip-autocomplete': {
      '& > ul': { fontFamily: 'inherit' },
      '& > ul > li[aria-selected]': { backgroundColor: isDark ? '#04395e' : '#ddeeff' },
    },
    '.cm-completionDetail': { fontStyle: 'normal', color: p.comment, marginLeft: '8px' },
    '.cm-completionMatchedText': { textDecoration: 'none', color: isDark ? '#18a3ff' : '#0066bf' },
    '.cm-tooltip.cm-completionInfo': { padding: '0', border: '1px solid var(--color-line)', whiteSpace: 'normal' },
    '&.cm-focused .cm-matchingBracket': { backgroundColor: 'var(--color-fg-ghost)', borderRadius: '2px', padding: '1px', margin: '-1px' },
    '&.cm-focused .cm-nonmatchingBracket': { backgroundColor: isDark ? '#6e2121' : '#ffd6d6', borderRadius: '2px', padding: '1px', margin: '-1px' },
    '.cm-searchMatch': { backgroundColor: isDark ? '#6c401580' : '#f5d02480', borderRadius: '2px', padding: '1px', margin: '-1px' },
    '.cm-searchMatch.cm-searchMatch-selected': { backgroundColor: isDark ? '#9e6a03' : '#f5d024', borderRadius: '2px', padding: '1px', margin: '-1px' },
    '.cm-selectionMatch': { backgroundColor: isDark ? '#add6ff26' : '#add6ff44', borderRadius: '2px', padding: '1px', margin: '-1px' },
    '.cm-panel.cm-search': { padding: '0', backgroundColor: 'var(--color-surface-editor)', '& input, & button, & label': { margin: '0' } },
    '.cm-foldPlaceholder': { backgroundColor: 'transparent', border: 'none', color: p.comment },
    '.cm-stickyHeaders': {
      position: 'absolute',
      top: '0',
      left: '0',
      right: '0',
      zIndex: '3',
      pointerEvents: 'none',
      overflow: 'hidden',
    },
    '.cm-stickyHeadersRows': {
      display: 'flex',
      flexDirection: 'column',
    },
    '.cm-stickyHeaderRow': {
      pointerEvents: 'none',
      backgroundColor: 'var(--color-surface-editor)',
      borderBottom: '1px solid var(--color-line)',
    },
    '.cm-stickyHeaderRow .cm-editor': {
      backgroundColor: 'transparent',
    },
    '.cm-stickyHeaderRow .cm-content': {
      padding: '0',
    },
    '.cm-foldGutter .cm-gutterElement': {
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      cursor: 'pointer',
      paddingRight: '4px',
    },
    '.cm-highlightSpace:before': { color: isDark ? '#3b3b3b' : '#d0d0d0' },
    '.cm-trailingSpace': { backgroundColor: '#ff000040' },
    '.cm-panels': { backgroundColor: 'var(--color-surface-raised)', color: p.foreground },
    '.cm-panels.cm-panels-top': { borderBottom: 'none' },
    '.cm-panels.cm-panels-bottom': { borderTop: 'none' },
    '[data-editor-scrollbar="vertical"]': {
      position: 'absolute',
      top: '0',
      right: '0',
      width: '10px',
      height: '100%',
      backgroundColor: isDark
        ? 'color-mix(in srgb, var(--color-fg-ghost) 18%, transparent)'
        : 'color-mix(in srgb, var(--color-fg-ghost) 32%, transparent)',
      pointerEvents: 'auto',
      zIndex: '3',
      opacity: '1',
      isolation: 'isolate',
    },
    '[data-editor-scrollbar="horizontal"]': {
      position: 'absolute',
      left: '0',
      bottom: '0',
      height: '10px',
      width: '100%',
      backgroundColor: isDark
        ? 'color-mix(in srgb, var(--color-fg-ghost) 18%, transparent)'
        : 'color-mix(in srgb, var(--color-fg-ghost) 32%, transparent)',
      pointerEvents: 'auto',
      zIndex: '3',
      opacity: '1',
      isolation: 'isolate',
    },
    '[data-editor-scrollbar][data-scrollbar-scrollable="false"]': {
      opacity: '0',
    },
    '[data-scrollbar-markers="vertical"]': {
      position: 'absolute',
      top: '0',
      right: '1px',
      width: '8px',
      height: '100%',
      pointerEvents: 'none',
    },
    '[data-scrollbar-markers="vertical-split"]': {
      position: 'absolute',
      top: '0',
      right: '1px',
      width: '8px',
      height: '100%',
      pointerEvents: 'none',
    },
    '[data-scrollbar-markers="vertical-left"], [data-scrollbar-markers="vertical-right"]': {
      position: 'absolute',
      top: '0',
      bottom: '0',
      width: '50%',
    },
    '[data-scrollbar-markers="vertical-left"]': {
      left: '0',
    },
    '[data-scrollbar-markers="vertical-right"]': {
      right: '0',
    },
    '[data-scrollbar-thumb="vertical"], [data-scrollbar-thumb="horizontal"]': {
      position: 'absolute',
      borderRadius: '999px',
      backgroundColor: isDark
        ? 'color-mix(in srgb, var(--color-fg-ghost) 56%, transparent)'
        : 'color-mix(in srgb, #9ca3af 62%, transparent)',
      opacity: '0',
      mixBlendMode: isDark ? 'screen' : 'multiply',
      pointerEvents: 'auto',
      transition: 'opacity 140ms ease, background-color 140ms ease',
    },
    '[data-editor-scrollbar][data-scrollbar-visible="true"] [data-scrollbar-thumb]': {
      opacity: isDark ? '0.88' : '0.64',
    },
    '[data-editor-scrollbar]:hover [data-scrollbar-thumb]': {
      backgroundColor: isDark
        ? 'color-mix(in srgb, var(--color-fg-ghost) 68%, transparent)'
        : 'color-mix(in srgb, #6b7280 72%, transparent)',
      opacity: isDark ? '0.95' : '0.76',
    },
    '[data-editor-scrollbar][data-scrollbar-scrollable="false"] [data-scrollbar-thumb]': {
      opacity: '0',
    },
    '[data-scrollbar-thumb="vertical"]': {
      top: '0',
      left: '1px',
      width: '8px',
    },
    '[data-scrollbar-thumb="horizontal"]': {
      top: '1px',
      left: '0',
      height: '8px',
    },
    '[data-editor-scrollbar-marker="selection"]': {
      backgroundColor: 'var(--color-on-accent)',
      opacity: '0.78',
    },
    '[data-editor-scrollbar-marker="search"]': {
      backgroundColor: isDark ? '#f5d024' : '#c79000',
      opacity: '0.6',
    },
    '[data-editor-scrollbar-marker="diagnostic"]': {
      backgroundColor: 'var(--color-on-warning)',
      opacity: '0.56',
    },
    '[data-editor-scrollbar-marker="diagnostic"][data-editor-scrollbar-severity="error"]': {
      backgroundColor: 'var(--color-on-danger)',
    },
    '[data-editor-scrollbar-marker="diagnostic"][data-editor-scrollbar-severity="warning"]': {
      backgroundColor: 'var(--color-on-warning)',
    },
    '[data-editor-scrollbar-marker="diagnostic"][data-editor-scrollbar-severity="info"]': {
      backgroundColor: 'var(--color-on-accent)',
    },
    '[data-editor-scrollbar-marker="diagnostic"][data-editor-scrollbar-severity="hint"]': {
      backgroundColor: 'var(--color-fg-faint)',
    },
    '[data-editor-scrollbar-marker="diff-added"]': {
      backgroundColor: 'var(--color-on-success)',
      opacity: '0.32',
    },
    '[data-editor-scrollbar-marker="diff-deleted"]': {
      backgroundColor: 'var(--color-on-danger)',
      opacity: '0.32',
    },
  }, { dark: isDark })
}

function buildHighlightStyle(p: CodeThemePalette) {
  return HighlightStyle.define([
    { tag: tags.keyword, color: p.keyword },
    { tag: [tags.controlKeyword, tags.moduleKeyword, tags.operatorKeyword], color: p.controlKeyword },
    { tag: tags.variableName, color: p.variable },
    { tag: tags.function(tags.variableName), color: p.function },
    { tag: tags.definition(tags.variableName), color: p.variable },
    { tag: tags.definition(tags.function(tags.variableName)), color: p.function },
    { tag: [tags.propertyName], color: p.variable },
    { tag: tags.function(tags.propertyName), color: p.function },
    { tag: [tags.typeName, tags.className, tags.namespace], color: p.type },
    { tag: tags.bool, color: p.keyword },
    { tag: [tags.number, tags.inserted], color: p.number },
    { tag: [tags.string, tags.special(tags.string)], color: p.string },
    { tag: tags.regexp, color: p.regexp },
    { tag: tags.comment, color: p.comment },
    { tag: tags.operator, color: p.operator },
    { tag: tags.punctuation, color: p.punctuation },
    { tag: tags.separator, color: p.punctuation },
    { tag: tags.angleBracket, color: p.punctuation },
    { tag: tags.tagName, color: p.tag },
    { tag: tags.attributeName, color: p.attribute },
    { tag: tags.attributeValue, color: p.string },
    { tag: [tags.atom, tags.constant(tags.variableName)], color: p.constant },
    { tag: tags.meta, color: p.keyword },
    { tag: tags.invalid, color: p.invalid },
    { tag: tags.link, color: p.constant, textDecoration: 'underline' },
    { tag: tags.heading, fontWeight: 'bold', color: p.keyword },
    { tag: tags.strong, fontWeight: 'bold' },
    { tag: tags.emphasis, fontStyle: 'italic' },
  ])
}

const themeCompartment = new Compartment()
const highlightCompartment = new Compartment()
const trackedViews = new Map<EditorView, () => void>()

function resolvePalette(host: EditorHost): { palette: CodeThemePalette; isDark: boolean } {
  const snapshot = host.theme.getSnapshot()
  if (snapshot.palette) {
    return { palette: snapshot.palette as CodeThemePalette, isDark: snapshot.mode === 'dark' }
  }
  const definition = codeThemes.find((item) => item.id === snapshot.codeThemeId) ?? codeThemes[0]!
  const variant = snapshot.mode === 'dark' ? definition.dark : definition.light
  return { palette: variant.palette, isDark: snapshot.mode === 'dark' }
}

function reconfigureView(view: EditorView, host: EditorHost) {
  const { palette, isDark } = resolvePalette(host)
  view.dispatch({
    effects: [
      themeCompartment.reconfigure(buildEditorTheme(palette, isDark)),
      highlightCompartment.reconfigure(syntaxHighlighting(buildHighlightStyle(palette))),
    ],
  })
}

export function wynkTheme(host: EditorHost) {
  const { palette, isDark } = resolvePalette(host)
  return [
    themeCompartment.of(buildEditorTheme(palette, isDark)),
    highlightCompartment.of(syntaxHighlighting(buildHighlightStyle(palette))),
  ]
}

export function trackEditorView(view: EditorView, host: EditorHost) {
  const unsubscribe = host.theme.subscribe(() => {
    reconfigureView(view, host)
  })
  trackedViews.set(view, unsubscribe)
}

export function untrackEditorView(view: EditorView) {
  trackedViews.get(view)?.()
  trackedViews.delete(view)
}
