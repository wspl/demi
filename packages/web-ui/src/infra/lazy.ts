import { shallowRef, type Ref } from 'vue'
import { reloadPage } from './build-reload'

// A part of the page loaded on first use (`file-previews.md` § Code), such
// as the code editor or the terminal: a chunk of its own, which the bundler
// keeps out of the files the page opens with as long as nothing imports it
// statically.

/** Where a view's chunk is: on its way, there, or failed with the reason. */
export type ChunkLoad<T> =
  | { phase: 'loading' }
  | { phase: 'ready'; chunk: T }
  | { phase: 'failed'; message: string }

export interface LazyChunk<T> {
  /** The chunk, loaded once. */
  load: () => Promise<T>
  /**
   * The chunk for a view that shows it, its load started as the view is set
   * up; `retry` is a failed load's Retry.
   */
  use: () => { load: Readonly<Ref<ChunkLoad<T>>>; retry: () => void }
}

/**
 * The chunk `importer` brings, loaded on first use. A load that failed is
 * not tried again in the page: a browser keeps a module that failed to load
 * as failed for the page's life, so a failed load's Retry loads the page
 * again, which keeps a drafted message and the reader's place.
 */
export function lazyChunk<T>(importer: () => Promise<T>): LazyChunk<T> {
  let loaded: T | null = null
  let loading: Promise<T> | null = null
  function load(): Promise<T> {
    loading ??= importer().then((chunk) => {
      loaded = chunk
      return chunk
    })
    return loading
  }
  function use() {
    const state = shallowRef<ChunkLoad<T>>(loaded ? { phase: 'ready', chunk: loaded } : { phase: 'loading' })
    if (!loaded) {
      load().then(
        (chunk) => {
          state.value = { phase: 'ready', chunk }
        },
        (error: unknown) => {
          state.value = { phase: 'failed', message: error instanceof Error ? error.message : String(error) }
        },
      )
    }
    return { load: state, retry: reloadPage }
  }
  return { load, use }
}
