import { Compartment } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { tags } from '@lezer/highlight'
import { appThemeStore } from '../../theme/appTheme'
import { INTERACTED_CLASS } from '../activeLineVisibility'
import { codeTheme, type CodeThemePalette } from '../../theme/codeTheme'

function buildEditorTheme(p: CodeThemePalette, isDark: boolean) {
  return EditorView.theme({
    '&': {
      color: p.foreground,
      backgroundColor: 'var(--color-surface-editor)',
      border: 'none',
    },
    '&.cm-editor.cm-focused': { outline: 'none' },
    // Both code views are viewers: no caret, while the editor keeps the mouse
    // and the keyboard, so a selection starts anywhere in it (the gutter, past
    // a line's end, below the last line) and Mod-f finds.
    '.cm-content': {
      caretColor: 'transparent',
      fontFamily: 'inherit',
      padding: '12px 0 0',
    },
    '.cm-cursorLayer': { display: 'none' },
    // The app's code font (`styles/base.css`) in place of CodeMirror's bare
    // `monospace`, which leaves Arabic to a font that does not join it.
    '.cm-scroller': {
      fontFamily: 'var(--font-mono)',
      fontSize: '13px',
      lineHeight: '20px',
      boxSizing: 'border-box',
    },
    '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection': {
      backgroundColor: p.selection,
    },
    '.cm-selectionBackground': { borderRadius: '2px' },
    '.cm-activeLine': { backgroundColor: isDark ? '#ffffff0a' : '#00000008' },
    '.cm-activeLineGutter': { backgroundColor: 'transparent', color: p.activeLineNumber },
    [`&:not(.${INTERACTED_CLASS}) .cm-activeLine`]: { backgroundColor: 'transparent' },
    [`&:not(.${INTERACTED_CLASS}) .cm-activeLineGutter`]: { color: p.lineNumber },
    '.cm-gutters': { backgroundColor: 'var(--color-surface)', color: p.lineNumber, border: 'none' },
    '.cm-lineNumbers .cm-gutterElement': { paddingLeft: '12px' },
    '&.cm-focused .cm-matchingBracket': { backgroundColor: 'var(--color-fg-ghost)', borderRadius: '2px', padding: '1px', margin: '-1px' },
    '&.cm-focused .cm-nonmatchingBracket': { backgroundColor: isDark ? '#6e2121' : '#ffd6d6', borderRadius: '2px', padding: '1px', margin: '-1px' },
    '.cm-searchMatch': { backgroundColor: isDark ? '#6c401580' : '#f5d02480', borderRadius: '2px', padding: '1px', margin: '-1px' },
    '.cm-searchMatch.cm-searchMatch-selected': { backgroundColor: isDark ? '#9e6a03' : '#f5d024', borderRadius: '2px', padding: '1px', margin: '-1px' },
    '.cm-selectionMatch': { backgroundColor: isDark ? '#add6ff26' : '#add6ff44', borderRadius: '2px', padding: '1px', margin: '-1px' },
    '.cm-foldPlaceholder': { backgroundColor: 'transparent', border: 'none', color: p.comment },
    '.cm-foldGutter .cm-gutterElement': {
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      cursor: 'pointer',
      paddingRight: '4px',
    },
    // The find bar draws its own background and border.
    '.cm-panels': { backgroundColor: 'transparent', color: 'inherit' },
    '.cm-panels.cm-panels-bottom': { borderTop: 'none' },
    // The ruler of marks under the vertical scrollbar (`scrollbars/ruler.ts`).
    '[data-scrollbar-markers="vertical"], [data-scrollbar-markers="vertical-split"]': {
      position: 'absolute',
      top: '0',
      right: '1px',
      width: '8px',
      height: '100%',
      pointerEvents: 'none',
    },
    '[data-scrollbar-markers][data-scrollbar-scrollable="false"]': {
      opacity: '0',
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
    '[data-editor-scrollbar-marker="selection"]': {
      backgroundColor: 'var(--color-on-accent)',
      opacity: '0.78',
    },
    '[data-editor-scrollbar-marker="search"]': {
      backgroundColor: isDark ? '#f5d024' : '#c79000',
      opacity: '0.6',
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

/** The palette for the page's mode, from the app's theme. */
function currentPalette(): { palette: CodeThemePalette; isDark: boolean } {
  const isDark = appThemeStore.state.mode === 'dark'
  return { palette: codeTheme[isDark ? 'dark' : 'light'].palette, isDark }
}

function reconfigureView(view: EditorView) {
  const { palette, isDark } = currentPalette()
  view.dispatch({
    effects: [
      themeCompartment.reconfigure(buildEditorTheme(palette, isDark)),
      highlightCompartment.reconfigure(syntaxHighlighting(buildHighlightStyle(palette))),
    ],
  })
}

/** The editor's look and its syntax colors, in the page's mode. */
export function editorTheme() {
  const { palette, isDark } = currentPalette()
  return [
    themeCompartment.of(buildEditorTheme(palette, isDark)),
    highlightCompartment.of(syntaxHighlighting(buildHighlightStyle(palette))),
  ]
}

/** Follows the page's mode until `untrackEditorView`. */
export function trackEditorView(view: EditorView) {
  trackedViews.set(view, appThemeStore.subscribe(() => reconfigureView(view)))
}

export function untrackEditorView(view: EditorView) {
  trackedViews.get(view)?.()
  trackedViews.delete(view)
}
