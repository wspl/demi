import type { ProviderErrorDiagnostics } from '@demicodes/protocol'
import { codeText } from '../infra/errors'
import { formatRetryAt } from '../ui/dates'
import type { SentenceText } from '../ui/ui-text'
import { t } from './locales/text'

/**
 * The headline of a failed turn (`localization.md` § Text from the
 * backend): the catalogue's words for its failure code, such as
 * `usage_limit`, and, when the provider names the moment it works again,
 * that moment counted from `now`: “You’ve reached your usage limit. Resets
 * at 9:00 PM.” A code the catalogue has no words for, as a vendor's own,
 * and a failure without a code read “The turn failed.”; what the vendor
 * said shows below it, untranslated.
 */
export function failureHeadline(code: string | null | undefined, retryAt: string | null, now: Date): SentenceText {
  const failure = (code ? codeText(code) : null) ?? t('transcript.turnFailed')
  return retryAt === null
    ? failure
    : t('transcript.failureResets', { failure, reset: formatRetryAt('Resets', retryAt, now) })
}

/**
 * What a failure record shows under its headline: the vendor's response
 * as it arrived, indented when it is JSON, behind Provider Response, for a
 * failure a provider reported; and the record's own text in the body only
 * for a failure the page has no words for that no provider reported, as a
 * record an older release wrote.
 */
export function failureSource(
  message: string,
  code: string | null | undefined,
  diagnostics: ProviderErrorDiagnostics | undefined,
): { raw: string | null; detail: string | null } {
  if (diagnostics) {
    return { raw: diagnostics.upstream ? prettyUpstream(diagnostics.upstream) : message.trim() || null, detail: null }
  }
  const worded = code ? codeText(code) !== null : false
  return { raw: null, detail: worded ? null : message.trim() || null }
}

/** What the copy button puts on the clipboard: the upstream message and every diagnostic. */
export function errorReportText(
  message: string,
  code: string | null | undefined,
  diagnostics: ProviderErrorDiagnostics | undefined
): string {
  const lines = [message]
  if (code)
    lines.push(`code: ${code}`)
  if (diagnostics) {
    const fields: Array<[string, string | number | undefined]> = [
      ['source', diagnostics.source],
      ['http', diagnostics.httpStatus],
      ['provider code', diagnostics.providerCode],
      ['request', diagnostics.clientRequestId],
      ['provider request', diagnostics.providerRequestId],
      ['response', diagnostics.providerResponseId],
    ]
    for (const [key, value] of fields) {
      if (value !== undefined && value !== '')
        lines.push(`${key}: ${value}`)
    }
    if (diagnostics.upstream)
      lines.push('', 'upstream:', prettyUpstream(diagnostics.upstream))
  }
  return lines.join('\n')
}

/** The provider's response as it arrived, indented when it is JSON, for a reader's eyes. */
export function prettyUpstream(upstream: string): string {
  try {
    return JSON.stringify(JSON.parse(upstream), null, 2)
  } catch {
    return upstream
  }
}
