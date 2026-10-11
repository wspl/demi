import { Compartment, type Extension } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import { syntaxHighlighting } from '@codemirror/language'
import { appThemeStore } from '../../theme/appTheme'
import { INTERACTED_CLASS } from '../activeLineVisibility'
import { codeTheme, type CodeThemePalette } from '../../theme/codeTheme'
import { codeHighlightStyle } from './highlightStyle'

function buildEditorTheme(p: CodeThemePalette, isDark: boolean) {
  return EditorView.theme({
    '&': {
      color: p.foreground,
      backgroundColor: 'var(--color-surface-editor)',
      border: 'none',
    },
    '&.cm-editor.cm-focused': { outline: 'none' },
    // Both code views are read-only, but read with the keyboard as well as
    // the mouse (`file-previews.md` § Code): focused, they blink a caret in
    // the theme's color, which the arrows move and Shift extends a selection
    // from; without the focus the editor draws none.
    '.cm-content': {
      caretColor: p.caret,
      fontFamily: 'inherit',
      padding: '12px 0 0',
    },
    '.cm-cursor, .cm-dropCursor': { borderLeftColor: p.caret },
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

const themeCompartment = new Compartment()
const highlightCompartment = new Compartment()
const trackedViews = new Map<EditorView, () => void>()

/** The editor's look and its syntax colors in the page's mode. */
function currentTheme(): { look: Extension; colors: Extension } {
  const mode = appThemeStore.state.mode
  return {
    look: buildEditorTheme(codeTheme[mode].palette, mode === 'dark'),
    colors: syntaxHighlighting(codeHighlightStyle(mode)),
  }
}

function reconfigureView(view: EditorView) {
  const { look, colors } = currentTheme()
  view.dispatch({
    effects: [themeCompartment.reconfigure(look), highlightCompartment.reconfigure(colors)],
  })
}

/** The editor's look and its syntax colors, in the page's mode. */
export function editorTheme(): Extension {
  const { look, colors } = currentTheme()
  return [themeCompartment.of(look), highlightCompartment.of(colors)]
}

/** Follows the page's mode until `untrackEditorView`. */
export function trackEditorView(view: EditorView) {
  trackedViews.set(view, appThemeStore.subscribe(() => reconfigureView(view)))
}

export function untrackEditorView(view: EditorView) {
  trackedViews.get(view)?.()
  trackedViews.delete(view)
}
