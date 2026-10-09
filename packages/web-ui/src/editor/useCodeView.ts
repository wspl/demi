import { onBeforeUnmount, onMounted, watch, type Ref } from 'vue'
import { ChangeSet, EditorState, type Extension, type StateEffect, type Text, type TransactionSpec } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import { getOriginalDoc, originalDocChangeEffect } from '@codemirror/merge'
import { baseName } from '@demicodes/utils'
import { loadFileLanguage } from './language'
import { editorTheme, trackEditorView, untrackEditorView } from './theme/cmTheme'

/** What a code view shows, read reactively: its text, and a diff's original. */
export interface CodeViewFile {
  /** The file's path, which selects its language; read once. */
  path: string
  text: string
  /** A unified diff's other side, which its merge view compares against. */
  original?: string
}

export interface CodeViewOptions {
  /** Where the view opens scrolled to: a snapshot an earlier view of the file took. */
  scrollTo?: StateEffect<unknown> | null
  /** Hears where the view was scrolled to as it goes. */
  left?(snapshot: StateEffect<unknown>): void
}

/**
 * The one change that turns `doc` into `next`: the stretch between their
 * common start and end, so what lies around it, folded, selected or
 * scrolled to, stays where it is. Null when they are the same.
 */
function replacement(doc: Text, next: string): { from: number; to: number; insert: string } | null {
  const current = doc.toString()
  if (current === next)
    return null
  const shorter = Math.min(current.length, next.length)
  let start = 0
  while (start < shorter && current.charCodeAt(start) === next.charCodeAt(start))
    start += 1
  let end = 0
  while (end < shorter - start && current.charCodeAt(current.length - 1 - end) === next.charCodeAt(next.length - 1 - end))
    end += 1
  return { from: start, to: current.length - end, insert: next.slice(start, next.length - end) }
}

/**
 * What turns the view's `state` into `file`: the text's change, and a
 * diff's new original, in one transaction; null when it shows them already.
 */
export function codeViewUpdate(state: EditorState, file: CodeViewFile): TransactionSpec | null {
  const change = replacement(state.doc, file.text)
  const original = file.original === undefined ? null : getOriginalDoc(state)
  const originalChange = original && replacement(original, file.original!)
  if (!change && !originalChange)
    return null
  return {
    ...(change ? { changes: change } : {}),
    ...(original && originalChange
      ? { effects: originalDocChangeEffect(state, ChangeSet.of(originalChange, original.length)) }
      : {}),
  }
}

/**
 * A read-only code view in `container` for the component's lifetime: one
 * file's text in the app's code theme, colored by the file's language. A new
 * text replaces the old in place, as an editor reloads a file it has not
 * changed: the lines the user is looking at stay where they are on screen,
 * and the selection and the folds outside what changed stay. The view is
 * built once the language has loaded, because a diff colors its removed
 * lines only as it builds them; `extensions` may be a function, which then
 * builds them at that moment from what the file is then.
 */
export function useCodeView(
  container: Readonly<Ref<HTMLElement | undefined>>,
  file: CodeViewFile,
  extensions: Extension | (() => Extension),
  options: CodeViewOptions = {},
): void {
  let view: EditorView | null = null
  let unmounted = false

  onMounted(async () => {
    const language = await loadFileLanguage(file.path)
    if (unmounted)
      return
    view = new EditorView({
      parent: container.value,
      state: EditorState.create({
        doc: file.text,
        extensions: [
          EditorState.readOnly.of(true),
          EditorState.tabSize.of(2),
          // CodeMirror makes the text a read-only, multi-line textbox; the file's name names it.
          EditorView.contentAttributes.of({ 'aria-label': baseName(file.path) }),
          editorTheme(),
          language,
          typeof extensions === 'function' ? extensions() : extensions,
        ],
      }),
      ...(options.scrollTo ? { scrollTo: options.scrollTo } : {}),
    })
    trackEditorView(view)
  })

  // Both sides of a diff arrive together and land in one transaction. The
  // view goes back to the place it was scrolled to, carried through the
  // change (a later spec's effects map through the earlier one's changes),
  // as an editor keeps the line at the top of the window where it is when
  // the file changes above it; the stretches a new version folds or unfolds
  // above would otherwise move it. A view out of sight, as in a tab not
  // selected, has no place on screen to keep: its scroll reads as its top.
  watch([() => file.text, () => file.original], () => {
    const update = view && codeViewUpdate(view.state, file)
    if (view && update)
      view.dispatch(update, view.inView ? { effects: view.scrollSnapshot() } : {})
  })

  onBeforeUnmount(() => {
    unmounted = true
    if (!view)
      return
    options.left?.(view.scrollSnapshot())
    untrackEditorView(view)
    view.destroy()
    view = null
  })
}
