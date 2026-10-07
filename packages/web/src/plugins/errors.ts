import { PluginCallError } from '@demicodes/web-ui/plugins/page'
import { ApiError } from '../api/client'

/** A refusal the backend answered, as a page sees it: the plugin's reason, or the backend's code. */
function refusal(error: ApiError): PluginCallError {
  return new PluginCallError(error.reason ?? error.code ?? `status_${error.status}`, error.message)
}

/**
 * A failed call as a page sees it: a refusal with its reason, anything else,
 * such as an abort, as it was. A call never fails for the connection: it
 * waits for the backend instead.
 */
export function callFailure(error: unknown): unknown {
  return error instanceof ApiError ? refusal(error) : error
}

/** Why a read failed, as a page shows it beside what it last read. */
export function readFailure(error: unknown): PluginCallError {
  if (error instanceof ApiError) {
    return refusal(error)
  }
  return new PluginCallError('failed', error instanceof Error ? error.message : String(error))
}
