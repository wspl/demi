import { onBeforeUnmount, onMounted, type Ref } from 'vue'
import { EditorState, type Extension } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import { loadFileLanguage } from './language'
import { editorTheme, trackEditorView, untrackEditorView } from './theme/cmTheme'

/**
 * A read-only code view in `container` for the component's lifetime: one
 * file's text in the app's code theme, colored by the file's language.
 * Another text needs another component. The view is built once the language
 * has loaded, because a diff colors its removed lines only as it builds them.
 */
export function useCodeView(
  container: Readonly<Ref<HTMLElement | undefined>>,
  file: { path: string; text: string },
  extensions: Extension,
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
          editorTheme(),
          language,
          extensions,
        ],
      }),
    })
    trackEditorView(view)
  })

  onBeforeUnmount(() => {
    unmounted = true
    if (!view)
      return
    untrackEditorView(view)
    view.destroy()
    view = null
  })
}
