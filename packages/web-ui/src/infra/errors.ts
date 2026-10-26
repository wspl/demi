import { find } from '../locales/text'
import { t } from './locales/text'
import { formatBytes } from '../files/format'
import { shownToast, showToast, showToastRequestId } from './toast'
import type { HeadlineText, SentenceText } from '../ui/ui-text'

export interface ReportErrorOptions {
  /**
   * Which failure this is, as a stable English ID that does not change with the
   * user's language, such as `failures.saveDrafts`: the i18n key of the title
   * where the title has one. It is what the log says of the failure, so a query
   * finds it by site; the title, which is in the user's words, is never logged.
   */
  site: string
  /** The conversation the failure is about, when the page knows it. */
  conversation?: string
  /**
   * Whether a toast tells of it. Left out, a defect gets one when the user
   * chose to see them (`showUnexpectedErrors`) and a failure of any other kind
   * none; `false` never shows one, for a report that is a diagnostic.
   */
  userVisible?: boolean
  /**
   * The page's own defect even when a request names the error, as with an error
   * nothing handled (`reportDefect`): it is logged, and the toast keeps the
   * request's ID.
   */
  defect?: boolean
  expected?: boolean
  /** Where it failed, such as what of a value did not read with its schema; never a value the page held. */
  detail?: string
  /**
   * What the toast says in place of the error's own message, when that
   * message is technical: the error's message then reaches only the log.
   */
  message?: SentenceText
}

/**
 * The report of a failure the page found itself, which the app writes as an error
 * line of the page's log (`log.ts`): what failed and where, never a value the page
 * held.
 */
export interface PageErrorReport {
  /** `ReportErrorOptions.site`. */
  site: string
  /** The error's own message, in English, without the query of any address. */
  message: string
  detail: string
  /** The error's `code` (`errorCode`), or null. */
  code: string | null
  /** The error's class, such as `TypeError` or `ApiError`. */
  errorClass: string
  /** The request the backend named, when one did. */
  requestId: string | null
  conversation?: string
}

/**
 * Writes a report and answers the ID of the request that carried it, or
 * null when it did not get in.
 */
export type PageErrorReporter = (report: PageErrorReport) => Promise<string | null>

let reporter: PageErrorReporter | null = null

/** Writes the page's own failures with `report`, which the app supplies; returns the undo. */
export function reportPageErrorsWith(report: PageErrorReporter): () => void {
  reporter = report
  return () => {
    if (reporter === report) {
      reporter = null
    }
  }
}

let showDefects = false

/**
 * Whether a defect no caller shows gets a toast with the ID of its line in the
 * log, the choice of Settings › General. It is the user's, so a defect is
 * logged whatever it says.
 */
export function showUnexpectedErrors(show: boolean): void {
  showDefects = show
}

/**
 * The ID of the backend request `error` came from, which the page shows with it:
 * an answer's, or a refused frame's lane's.
 *
 * It is null for a failure the page found itself, and for an answer something in
 * front of the backend gave.
 */
export function requestIdOf(error: unknown): string | null {
  return error instanceof Error && 'requestId' in error && typeof error.requestId === 'string'
    ? error.requestId
    : null
}

/**
 * An error as the page shows it in a form or a row: its words, and the ID of the
 * request it came from, which goes beside them.
 *
 * The words are made as they are read, from the failure the error keeps, so a
 * language chosen while it shows rewords it.
 */
export interface ShownError {
  readonly message: SentenceText
  requestId: string | null
}

/**
 * Marks a failure the page found itself with the `code` its text is looked up by,
 * `errors.<code>`, and the `params` that text is filled with, as a refusal of the
 * backend carries them.
 *
 * The message stays English, for the log; what the page shows is
 * the catalogue's.
 */
export function coded<E extends Error>(error: E, code: string, params: Readonly<Record<string, string | number>> = {}): E {
  return Object.assign(error, { code, params })
}

/** The code a refusal or a failure the page coded carries (`coded`), or null. */
export function errorCode(error: unknown): string | null {
  return error instanceof Error && 'code' in error && typeof error.code === 'string' ? error.code : null
}

/**
 * Marks `error` as a failure of the page's environment, not a defect. Examples:
 *
 * - The network brought no answer.
 * - The browser's storage refused, as with a full quota.
 * - A part of the page did not load.
 *
 * `reportError` tells of it as of any failure and reports it to no one. Mark it
 * where the failure comes from, which alone knows what it is. Answers `error`.
 */
export function environmental<E>(error: E): E {
  if (error instanceof Error) {
    Object.assign(error, { environment: true })
  }
  return error
}

/**
 * How the browsers reject a dynamic import whose module did not download:
 * Chrome's, Firefox's and Safari's TypeError, and Vite's own when a chunk's
 * stylesheet did not load.
 *
 * A loader such as CodeMirror's language data imports and builds in one promise,
 * and the browsers give no other way to tell a download that failed from a module
 * that threw.
 */
const CHUNK_DOWNLOAD_FAILED = /dynamically imported module|Importing a module script failed|Unable to preload CSS/

/**
 * `error` of loading a part of the page, marked as the environment's
 * (`environmental`) when the part did not download; a module that threw as
 * it loaded or was built is a defect, and stays one.
 */
export function chunkLoadFailure(error: unknown): unknown {
  return error instanceof Error && CHUNK_DOWNLOAD_FAILED.test(error.message) ? environmental(error) : error
}

/** Whether `error` is the environment's (`environmental`). */
function isEnvironmental(error: unknown): boolean {
  return error instanceof Error && 'environment' in error && error.environment === true
}

/**
 * The catalogue's text for the code `code`, `errors.<code>`, filled with `params`
 * as the page writes each.
 *
 * - A value whose name ends in `Bytes` is written as a size: `{ limitBytes:
 *   1048576 }` as *1.0 MB*.
 * - A `count` chooses the plural form.
 * - A code the catalogue has no text for gives null, as a newer backend may send.
 */
export function codeText(code: string, params: Readonly<Record<string, unknown>> = {}): SentenceText | null {
  const named = Object.fromEntries(Object.entries(params).map(([name, value]) => [
    name,
    name.endsWith('Bytes') && typeof value === 'number' ? formatBytes(value) : value,
  ]))
  return find(`errors.${code}`, named)
}

/**
 * What the page says of `error`, the one place a failure becomes words:
 *
 * - A refusal: the catalogue text for its `code`, filled with its `params`
 *   ([`codeText`]).
 * - An answer that carries no code, as a proxy's: the text of its HTTP status.
 * - A code the catalogue has no text for, as a newer backend may send, and any
 *   other failure: its own `message`.
 */
export function errorMessage(error: unknown): SentenceText {
  if (!(error instanceof Error)) {
    return String(error)
  }
  const code = errorCode(error)
  if (code !== null) {
    const params = 'params' in error && typeof error.params === 'object' && error.params !== null
      ? Object.fromEntries(Object.entries(error.params))
      : {}
    const worded = codeText(code, params)
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
 * `error` as the form the user filled shows it.
 *
 * A refusal whose `code` is one of `corrections`' is one the user corrects there,
 * such as a wrong current password or an address already taken. It names no
 * request, since nothing about it needs tracing
 * (`crates/backend-http/src/request_id.rs`), and says the words `corrections`
 * gives its code, or its own when that is null.
 *
 * Any other failure names its request.
 */
export function formError(
  error: unknown,
  corrections: Readonly<Record<string, SentenceText | null | undefined>>,
): ShownError {
  const code = errorCode(error)
  if (code !== null && code in corrections) {
    return worded(() => corrections[code] ?? errorMessage(error), null)
  }
  return shownError(error)
}

/**
 * An address's query in a report's message: after the `?` of an absolute URL or a
 * path, up to the next space, quote or fragment.
 *
 * A failure's message can quote the address it asked, whose query can carry a
 * file's path or a token. The report keeps the address's path alone.
 */
const QUERY = /((?:[a-z][a-z0-9+.-]*:\/\/|(?<![\w.])\/)[^\s?#"'`]*)\?[^\s#"'`]*/gi

/** A quoted piece of a message, which a `SyntaxError` of the browser takes from the text it could not read. */
const QUOTED = /(["'`]).*?\1/g

/** `message` of a `SyntaxError` with its quoted pieces taken out, which are the text the browser could not read. */
export function withoutQuoted(message: string): string {
  return message.replace(QUOTED, '…')
}

/** `text` with the query of every address in it removed. */
function withoutQueries(text: string): string {
  return text.replace(QUERY, '$1')
}

/**
 * What a line of the log says of `error`: its class, its `code` and the request
 * the backend named, as scalars, and its message without the query of any
 * address. A line says what failed, never a value the page held, so only these
 * go in; `null` fields are kept so that a query reads the same column each time.
 */
export function errorFields(error: unknown): { error_class: string; code: string | null; request_id: string | null; message: string } {
  // A `SyntaxError` of the browser quotes the text it could not read.
  const message = error instanceof SyntaxError ? withoutQuoted(error.message) : error instanceof Error ? error.message : String(error)
  return {
    error_class: error instanceof Error ? error.name : typeof error,
    code: errorCode(error),
    request_id: requestIdOf(error),
    message: withoutQueries(message),
  }
}

/** Writes a report of the page's own failure; the request ID when one carried it. */
async function sendReport(report: PageErrorReport): Promise<string | null> {
  if (!reporter) {
    return null
  }
  try {
    return await reporter({ ...report, message: withoutQueries(report.message) })
  } catch (error) {
    // A report that did not get in leaves the failure on the console
    // alone; reporting that would only repeat it.
    console.error('[demi] The error report was not sent:', error)
    return null
  }
}

/**
 * Tells of a failure: in a toast when the user should see it, with the ID of the
 * request it came from. A failure the page found itself is written to the log;
 * nothing goes to the console, which no one reads when a user reports a problem.
 *
 * A defect no request names, one the page found itself, is written to the log as an
 * error line, and its toast shows the ID of the request that carried the line once
 * it is in. A failure of the environment (`environmental`) is not reported.
 *
 * Every occurrence of a defect writes its line, but the same failure does not
 * stack toasts: while its toast is shown, a repeat shows nothing more, and a
 * repeat of a failure a request names gives the toast that request's ID.
 */
export function reportError(
  title: HeadlineText,
  error: unknown,
  options: ReportErrorOptions,
): void {
  const message = error instanceof Error ? error.message : String(error)
  const detail = options.detail ?? detailOf(error)
  const requestId = requestIdOf(error)
  const defect = !options.expected && !isEnvironmental(error) && (requestId === null || options.defect === true)
  const key = `${title}\n${message}\n${detail}`
  const reported = defect
    ? sendReport({
      site: options.site,
      message,
      detail,
      code: errorCode(error),
      errorClass: error instanceof Error ? error.name : typeof error,
      requestId,
      conversation: options.conversation,
    })
    : null
  if (!(options.userVisible ?? (defect && showDefects)))
    return
  const shown = defect ? shownToast(key) : undefined
  if (shown !== undefined) {
    return
  }
  // What the page found itself has no words of the catalogue's: its body is the
  // error's own message, in English, on one line. A coded failure, or one a
  // caller shows, has words.
  const worded = options.userVisible === true || errorCode(error) !== null || options.message !== undefined
  const toast = showToast({
    title,
    message: () => options.message ?? (worded ? errorMessage(error) : message),
    tone: 'danger',
    oneLine: !worded,
    requestId,
    key: defect ? key : undefined,
  })
  if (requestId === null) {
    void reported?.then((lineId) => {
      if (lineId) {
        showToastRequestId(toast, lineId)
      }
    })
  }
}

/**
 * Tells of a defect of the page that no caller words: an error nothing caught, or
 * one a plugin's page reports as its own. Its toast is the catalogue's *Something
 * Went Wrong* over the error's message, `site` names it in the log, and `what`
 * says where it happened, in English, for the log.
 */
export function reportDefect(site: string, what: string, error: unknown): void {
  const requestId = requestIdOf(error)
  const where = requestId === null ? what : `${what} (request ${requestId})`
  reportError(t('unexpected.title'), error, { site, defect: true, detail: `${where}\n${detailOf(error)}` })
}
