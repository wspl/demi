import type { App } from 'vue'
import { isAbortError } from '@demicodes/utils'
import { chunkLoadFailure, environmental, reportDefect } from './errors'

/**
 * Tells of every JavaScript error nothing caught, through `reportError`, so that
 * each is logged and, when the user chose to see them, shown with its ID.
 *
 * ```text
 *   a handler or timer throws        --> window 'error'
 *   a promise rejects, no one waits  --> window 'unhandledrejection'
 *   a render, watcher or lifecycle   --> app.config.errorHandler
 *   throws inside Vue
 * ```
 *
 * What is not a defect of the page is left out:
 *
 * - A cancelled operation (`AbortError`).
 * - A browser's own notice that no code threw, such as ResizeObserver's, and an
 *   error of an extension's script, which a browser may hide as *Script error.*.
 * - A failure of the environment: a request that got no answer, a part of the
 *   page that did not download (`environmental`).
 */

/**
 * How the browsers reject a `fetch` that reached no server: Chrome's, Firefox's
 * and Safari's TypeError.
 */
const FETCH_FAILED = /^(Failed to fetch|Load failed|NetworkError when attempting to fetch resource\.?)$/

/** What ResizeObserver reports when a frame could not deliver everything; no code threw. */
const OBSERVER_NOTICE = /^ResizeObserver loop/

/** A script of a browser extension, in an error's file name or its stack. */
const EXTENSION_SCRIPT = /(chrome|moz|safari|safari-web)-extension:\/\//

/** What a browser says of an error in a script of another origin, hiding the error itself. */
const HIDDEN_ERROR = /^Script error\.?$/

/** A quoted piece of a message, which a `SyntaxError` of the browser takes from the text it could not read. */
const QUOTED = /(["'`]).*?\1/g

/** Whether an error is already being reported, so that a failure of the report itself ends there. */
let reporting = false

/**
 * `thrown` as an error: anything else thrown is a value the page held, of which
 * only its kind goes on. A `SyntaxError` loses the pieces of its message in quotes,
 * which are the text it could not read.
 */
function asError(thrown: unknown): Error {
  if (thrown instanceof SyntaxError) {
    const message = thrown.message.replace(QUOTED, '…')
    const clean = new SyntaxError(message)
    clean.stack = thrown.stack?.replace(thrown.message, message)
    return clean
  }
  return thrown instanceof Error ? thrown : new Error(`A non-Error ${typeof thrown} was thrown.`)
}

/** Whether `thrown` says the environment failed, which marks it so. */
function classify(thrown: Error): Error {
  if (thrown instanceof TypeError && FETCH_FAILED.test(thrown.message)) {
    return environmental(thrown)
  }
  return chunkLoadFailure(thrown) as Error
}

function reportUncaught(thrown: unknown, where: string): void {
  if (reporting || isAbortError(thrown)) {
    return
  }
  reporting = true
  try {
    const error = classify(asError(thrown))
    if (!EXTENSION_SCRIPT.test(error.stack ?? '')) {
      reportDefect(where, error)
    }
  } finally {
    reporting = false
  }
}

/** Starts telling of what nothing caught, in `app`'s components and in the page; returns the undo. */
export function reportUncaughtErrors(app: App): () => void {
  const failed = (event: ErrorEvent) => {
    const hidden = event.error == null && HIDDEN_ERROR.test(event.message)
    if (hidden || OBSERVER_NOTICE.test(event.message) || EXTENSION_SCRIPT.test(event.filename)) {
      return
    }
    reportUncaught(event.error ?? event.message, 'an error nothing caught')
  }
  const rejected = (event: PromiseRejectionEvent) => {
    reportUncaught(event.reason, 'a promise rejected that nothing waited for')
  }
  window.addEventListener('error', failed)
  window.addEventListener('unhandledrejection', rejected)
  app.config.errorHandler = (error, instance, info) => {
    const component = instance?.$options.__name ?? instance?.$options.name
    reportUncaught(error, `Vue reported an error in ${component ?? 'a component'}: ${info}`)
  }
  return () => {
    window.removeEventListener('error', failed)
    window.removeEventListener('unhandledrejection', rejected)
    app.config.errorHandler = undefined
  }
}
