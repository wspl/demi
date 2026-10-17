import type { ProviderErrorDiagnostics } from '@demicodes/protocol'
import { z } from 'zod'
import { codeText } from '../infra/errors'
import { formatRetryAt } from '../ui/dates'
import type { SentenceText } from '../ui/ui-text'
import { t } from './locales/text'
import type { CredentialKind } from './credential-kinds'

/**
 * The headline of a failed turn (`localization.md` § Text from the
 * backend): the catalogue's words for its failure code, such as
 * `usage_limit`, and, when the provider names the moment it works again,
 * that moment counted from `now`: “You’ve reached your usage limit. Resets
 * at 9:00 PM.” A code the catalogue has no words for, as a vendor's own,
 * and a failure without a code read “The turn failed.”; what the vendor
 * said shows below it, untranslated.
 */
export function failureHeadline(
  code: string | null | undefined,
  retryAt: string | null,
  now: Date,
  credential: CredentialKind | null = null,
): SentenceText {
  // A key is never called a sign-in (`failures-and-recovery.md` § Failure codes).
  const words = code === 'auth_expired' && credential === 'api_key'
    ? t('transcript.apiKeyRefused')
    : code ? codeText(code) : null
  const failure = words ?? t('transcript.turnFailed')
  return retryAt === null
    ? failure
    : t('transcript.failureResets', { failure, reset: formatRetryAt('Resets', retryAt, now) })
}

/**
 * The headline of a failure's record in the transcript: while the vendor
 * has the account the request used signed out, the record says so in its
 * own words, and the dock offers Open Providers; otherwise the code's
 * headline, worded by the credential the entry holds (`credential`, null
 * when the page does not know the entry).
 */
export function recordHeadline(
  code: string | null | undefined,
  facts: {
    retryAt: string | null
    signedOut: { provider: string; account: string } | null
    credential: CredentialKind | null
  },
  now: Date,
): SentenceText {
  return facts.signedOut
    ? t('transcript.signedOut', { provider: facts.signedOut.provider, account: facts.signedOut.account })
    : failureHeadline(code, facts.retryAt, now, facts.credential)
}

/**
 * The record of a request the vendor answered with a failure status
 * (`failures-and-recovery.md` § Diagnostics): its status, every header as a
 * `[name, value]` pair, and the body as received.
 */
const httpRecordSchema = z.object({
  status: z.int(),
  headers: z.array(z.tuple([z.string(), z.string()])),
  body: z.string(),
})

/**
 * What a failure record shows under its headline: the vendor's response
 * behind Provider Response, for a failure a provider reported; and the
 * record's own text in the body only for a failure the page has no words
 * for that no provider reported, as a record an older release wrote. An
 * `http` record shows what the vendor said first, *HTTP 403* and its body,
 * indented when it is JSON, and its headers apart, which the notice folds
 * under Headers; any other record shows as it arrived, indented when it is
 * JSON.
 */
export function failureSource(
  message: string,
  code: string | null | undefined,
  diagnostics: ProviderErrorDiagnostics | undefined,
): { raw: string | null; headers: string | null; detail: string | null } {
  if (diagnostics) {
    const upstream = diagnostics.upstream
    if (!upstream) {
      return { raw: message.trim() || null, headers: null, detail: null }
    }
    const record = diagnostics.source === 'http' ? parsedHttpRecord(upstream) : null
    if (!record) {
      return { raw: prettyUpstream(upstream), headers: null, detail: null }
    }
    const said = record.body.trim()
    return {
      raw: said ? `HTTP ${record.status}\n\n${prettyUpstream(said)}` : `HTTP ${record.status}`,
      headers: record.headers.length > 0 ? record.headers.map(([name, value]) => `${name}: ${value}`).join('\n') : null,
      detail: null,
    }
  }
  const worded = code ? codeText(code) !== null : false
  return { raw: null, headers: null, detail: worded ? null : message.trim() || null }
}

/** `upstream` read as an `http` record; null when it is not one, and it then shows as it arrived. */
function parsedHttpRecord(upstream: string): z.infer<typeof httpRecordSchema> | null {
  try {
    const parsed = httpRecordSchema.safeParse(JSON.parse(upstream))
    return parsed.success ? parsed.data : null
  } catch {
    // Not JSON: a record of another shape, shown as it arrived.
    return null
  }
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
