import { PluginCallError } from '@demicodes/web-ui/plugins/page'
import { ApiError, UnreadableResponse } from '../api/client'

/** A refusal the backend answered, as a page sees it: the plugin's reason, or the backend's code, and the answer's status. */
function refusal(error: ApiError): PluginCallError {
  return new PluginCallError(
    error.reason ?? error.code ?? `status_${error.status}`,
    error.message,
    { type: 'status', status: error.statusLine },
    { requestId: error.requestId },
  )
}

/** An answer the page could not read, as a page sees it. */
function unreadable(error: UnreadableResponse): PluginCallError {
  return new PluginCallError('unreadable_answer', error.message, { type: 'unreadable' }, { detail: error.detail })
}

/**
 * A failed call as a page sees it: a refusal with its reason, an answer the
 * page could not read, and anything else, such as an abort, as it was. A
 * call never fails for the connection: it waits for the backend instead.
 */
export function callFailure(error: unknown): unknown {
  if (error instanceof ApiError) {
    return refusal(error)
  }
  return error instanceof UnreadableResponse ? unreadable(error) : error
}

/** Why a read failed, as a page shows it beside what it last read. */
export function readFailure(error: unknown): PluginCallError {
  if (error instanceof ApiError) {
    return refusal(error)
  }
  if (error instanceof UnreadableResponse) {
    return unreadable(error)
  }
  return new PluginCallError('failed', error instanceof Error ? error.message : String(error))
}
