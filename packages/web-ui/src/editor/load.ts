import { defineAsyncComponent } from 'vue'
import { chunkLoadFailure, reportError } from '../infra/errors'
import { lazyChunk } from '../infra/lazy'
import { t } from './locales/text'

// What the page holds of the editor before it arrives (`file-previews.md`
// § Code): the editor, its languages and its highlighter are chunks of their
// own, loaded on first use, so a page that shows no code downloads none of
// them. Nothing outside the editor imports `./editor` or the highlighter
// statically; everything reaches them through here and `markdown/highlight.ts`.

/** The editor's components and the language data, as one chunk. */
export type Editor = typeof import('./editor')

const editorChunk = lazyChunk<Editor>(() => import('./editor'))

/** The editor, loaded once. */
export const loadEditor = editorChunk.load

/**
 * The File view's editor and the Change view's, each rendered once its
 * view's `useEditor` says the editor is there, so it shows at once.
 */
export const CodeEditor = defineAsyncComponent(() => loadEditor().then((editor) => editor.CodeEditor))
export const DiffEditor = defineAsyncComponent(() => loadEditor().then((editor) => editor.DiffEditor))

/**
 * The editor for a view that shows code, its load started as the view is
 * set up, so it travels while the view reads its file. `retry` is a failed
 * load's Retry, which loads the page again (`lazyChunk`).
 */
export const useEditor = editorChunk.use

/**
 * Tells that a language's syntax colors could not load, as the editor and a
 * message's code blocks both do: a language that did not download is the
 * environment's failure, one that threw a defect.
 */
export function reportSyntaxColorsUnavailable(error: unknown): void {
  reportError(t('syntaxColorsUnavailable'), chunkLoadFailure(error))
}
