import { defineAsyncComponent, shallowRef, type Ref } from 'vue'
import { reportError } from '../infra/errors'
import { t } from './locales/text'

// What the page holds of the editor before it arrives (`file-previews.md`
// § Code): the editor, its languages and its highlighter are chunks of their
// own, loaded on first use, so a page that shows no code downloads none of
// them. Nothing outside the editor imports `./editor` or the highlighter
// statically; everything reaches them through here and `markdown/highlight.ts`.

/** The editor's components and the language data, as one chunk. */
export type Editor = typeof import('./editor')

let loaded: Editor | null = null
let loading: Promise<Editor> | null = null

/** The editor, loaded once; a load that failed is tried again by the next call. */
export function loadEditor(): Promise<Editor> {
  loading ??= import('./editor').then(
    (editor) => {
      loaded = editor
      return editor
    },
    (error: unknown) => {
      loading = null
      throw error
    },
  )
  return loading
}

/**
 * The File view's editor and the Change view's, each rendered once its
 * view's `useEditor` says the editor is there, so it shows at once.
 */
export const CodeEditor = defineAsyncComponent(() => loadEditor().then((editor) => editor.CodeEditor))
export const DiffEditor = defineAsyncComponent(() => loadEditor().then((editor) => editor.DiffEditor))

/** Where a view's editor is: on its way, there, or failed with the reason. */
export type EditorLoad =
  | { phase: 'loading' }
  | { phase: 'ready'; editor: Editor }
  | { phase: 'failed'; message: string }

/**
 * The editor for a view that shows code, its load started as the view is
 * set up, so it travels while the view reads its file. `retry` loads it
 * again after a failure.
 */
export function useEditor(): { load: Readonly<Ref<EditorLoad>>; retry: () => Promise<void> } {
  const load = shallowRef<EditorLoad>(loaded ? { phase: 'ready', editor: loaded } : { phase: 'loading' })
  async function retry(): Promise<void> {
    load.value = { phase: 'loading' }
    try {
      load.value = { phase: 'ready', editor: await loadEditor() }
    } catch (error) {
      load.value = { phase: 'failed', message: error instanceof Error ? error.message : String(error) }
    }
  }
  if (!loaded)
    void retry()
  return { load, retry }
}

/** Tells that a language's syntax colors could not load, as the editor and a message's code blocks both do. */
export function reportSyntaxColorsUnavailable(error: unknown): void {
  reportError(t('syntaxColorsUnavailable'), error)
}
