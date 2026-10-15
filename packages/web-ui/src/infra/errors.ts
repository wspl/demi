import { find } from '../locales/text'
import { showToast, showToastRequestId } from './toast'
import type { HeadlineText, SentenceText } from '../ui/ui-text'

export interface ReportErrorOptions {
  userVisible?: boolean
  expected?: boolean
  /** Where it failed, such as what of a value did not read with its schema; never a value the page held. */
  detail?: string
  /**
   * What the toast says in place of the error's own message, when that
   * message is technical: the error's message then reaches only the console.
   */
  message?: SentenceText
}

/**
 * The report of a failure the page found itself, which the backend logs
 * under the report's own request ID (`backend.md` § Request IDs and the
 * server log): what failed and where, never a value the page held.
 */
export interface PageErrorReport {
  title: string
  message: string
  detail: string
}

/**
 * Sends a report and answers the ID of the request that carried it, or
 * null when the backend refused it, as past thirty a minute.
 */
export type PageErrorReporter = (report: PageErrorReport) => Promise<string | null>

let reporter: PageErrorReporter | null = null

/** Sends the page's own failures with `report`, which the app supplies; returns the undo. */
export function reportPageErrorsWith(report: PageErrorReporter): () => void {
  reporter = report
  return () => {
    if (reporter === report) {
      reporter = null
    }
  }
}

/**
 * The ID of the backend request `error` came from, which the page shows
 * with it: an answer's, or a refused frame's socket's. Null for a failure
 * the page found itself, and for an answer something in front of the
 * backend gave.
 */
export function requestIdOf(error: unknown): string | null {
  return error instanceof Error && 'requestId' in error && typeof error.requestId === 'string'
    ? error.requestId
    : null
}

/**
 * An error as the page shows it in a form or a row: its words, and the ID
 * of the request it came from, which goes beside them. The words are made
 * as they are read, from the failure the error keeps, so a language chosen
 * while it shows rewords it.
 */
export interface ShownError {
  readonly message: SentenceText
  requestId: string | null
}

/**
 * Marks a failure the page found itself with the `code` its text is looked
 * up by, `errors.<code>`, and the `params` that text is filled with, as a
 * refusal of the backend carries them. The message stays English, for the
 * console and the report; what the page shows is the catalogue's.
 */
export function coded<E extends Error>(error: E, code: string, params: Readonly<Record<string, string | number>> = {}): E {
  return Object.assign(error, { code, params })
}

/**
 * What the page says of `error`, the one place a failure becomes words
 * (`localization.md` § Text from the backend): a refusal's catalogue text
 * for its `code`, filled with its `params`, such as `{ limit: 200 }`; for
 * an answer that carries no code, as a proxy's, the text of its HTTP
 * status; its own `message` for a code the catalogue has no text for, as a
 * newer backend may send, and for any other failure.
 */
export function errorMessage(error: unknown): SentenceText {
  if (!(error instanceof Error)) {
    return String(error)
  }
  if ('code' in error && typeof error.code === 'string') {
    const params = 'params' in error && typeof error.params === 'object' && error.params !== null
      ? Object.fromEntries(Object.entries(error.params))
      : {}
    const worded = find(`errors.${error.code}`, params)
    if (worded !== null) {
      return worded
    }
  } else if ('code' in error && error.code === null && 'status' in error && typeof error.status === 'number') {
    return find('errors.request_failed', { status: error.status }) ?? error.message
  }
  return error.message
}

/** `error` as the page shows it, in `message`'s words when given. */
export function shownError(error: unknown, message?: SentenceText): ShownError {
  return worded(() => message ?? errorMessage(error), requestIdOf(error))
}

/** A shown error whose words `words` makes each time they are read. */
function worded(words: () => SentenceText, requestId: string | null): ShownError {
  return {
    get message() {
      return words()
    },
    requestId,
  }
}

/**
 * Where `error` failed: its own `detail`, as an answer that did not read
 * says what of it did not (`schemaRefusal`), else its stack.
 */
function detailOf(error: unknown): string {
  if (!(error instanceof Error)) {
    return ''
  }
  return 'detail' in error && typeof error.detail === 'string' ? error.detail : error.stack ?? ''
}

/**
 * `error` as the form the user filled shows it. A refusal whose `code` is
 * one of `corrections`' is one the user corrects there, such as a wrong
 * current password or an address already taken: it names no request, since
 * nothing about it needs tracing (`backend.md` § Request IDs and the server
 * log), and says the words `corrections` gives its code, or its own when
 * that is null. Any other failure names its request.
 */
export function formError(
  error: unknown,
  corrections: Readonly<Record<string, SentenceText | null | undefined>>,
): ShownError {
  const code = error instanceof Error && 'code' in error && typeof error.code === 'string' ? error.code : null
  if (code !== null && code in corrections) {
    return worded(() => corrections[code] ?? errorMessage(error), null)
  }
  return shownError(error)
}

/**
 * An address's query in a report's message: after the `?` of an absolute
 * URL or a path, up to the next space, quote or fragment. A failure's
 * message can quote the address it asked, whose query can carry a file's
 * path or a token; the report keeps the address's path alone.
 */
const QUERY = /((?:[a-z][a-z0-9+.-]*:\/\/|(?<![\w.])\/)[^\s?#"'`]*)\?[^\s#"'`]*/gi

/** `text` with the query of every address in it removed. */
function withoutQueries(text: string): string {
  return text.replace(QUERY, '$1')
}

/** The backend's limits on a report's fields, in characters. */
const REPORT_LIMITS = { title: 200, message: 2000, detail: 16000 } as const

/** Sends a report of the page's own failure; the request ID when one carried it. */
async function sendReport(report: PageErrorReport): Promise<string | null> {
  if (!reporter) {
    return null
  }
  try {
    return await reporter({
      title: report.title.slice(0, REPORT_LIMITS.title),
      message: withoutQueries(report.message).slice(0, REPORT_LIMITS.message),
      detail: report.detail.slice(0, REPORT_LIMITS.detail),
    })
  } catch (error) {
    // A report that did not reach the backend leaves the failure on the
    // console alone; reporting that would only repeat it.
    console.error('[demi] The error report was not sent:', error)
    return null
  }
}

/**
 * Tells of a failure: on the console unless it was expected, and in a toast
 * when the user should see it, with the ID of the request it came from. A
 * failure no request names, one the page found itself, is reported to the
 * backend, and its toast shows the report's ID once the report is in.
 */
export function reportError(
  title: HeadlineText,
  error: unknown,
  options: ReportErrorOptions = {}
): void {
  const message = error instanceof Error ? error.message : String(error)
  if (!options.expected)
    console.error(`[demi] ${title}: ${message}`, options.detail ?? '')
  const requestId = requestIdOf(error)
  const reported = options.expected || requestId !== null
    ? null
    : sendReport({ title, message, detail: options.detail ?? detailOf(error) })
  if (!options.userVisible)
    return
  const toast = showToast({
    title,
    message: () => options.message ?? errorMessage(error),
    tone: 'danger',
    requestId,
  })
  void reported?.then((requestId) => {
    if (requestId) {
      showToastRequestId(toast, requestId)
    }
  })
}
